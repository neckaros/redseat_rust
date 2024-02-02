use axum::async_trait;
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};

use crate::model::{ModelController, ServerUser};
use crate::server::get_server_id;
use crate::tools::auth::verify;
use crate::{error::Error, Result};

const BEARER: &str = "Bearer ";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenParams {
    token: Option<String>
}





pub async fn mw_token_resolver(mc: State<ModelController>, headers: HeaderMap, query: Query<TokenParams>, mut req: Request, next: Next) -> Result<Response> {
    let token: Option<String> = match headers.get("AUTHORIZATION").and_then(|t| t.to_str().ok()) {
        Some(token) => Some(token.replace(BEARER, "")),
        None => match &query.token {
            Some(token) => Some(token.clone()),
            None => None,
        },
    };
    
    if let Some(token) = token {
        let server_id = get_server_id().await;
        let claims = verify(&token, &server_id)?;
        let user = mc.0.get_user(&claims.sub).await;
        println!("RETURN {:?}", user);
        if let Ok(user) = user {
            println!("INSERT {:?}", user);
            req.extensions_mut().insert(user);
        }

        
        
    }
    
    
    Ok(next.run(req).await)
}




#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ServerUser {
	type Rejection = Error;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self> {

		let server_user = parts
			.extensions
			.get::<ServerUser>().ok_or(Error::AuthFail)?;

        return  Ok(server_user.clone());
    }
}
