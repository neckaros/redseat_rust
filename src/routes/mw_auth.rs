use axum::async_trait;
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};

use crate::model::server::AuthMessage;
use crate::model::users::{ConnectedUser, GuestUser, UserRole};
use crate::model::ModelController;
use crate::server::get_server_id;
use crate::tools::auth::{verify, verify_local, ClaimsLocalType};
use crate::{error::Error, Result};

const BEARER: &str = "Bearer ";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenParams {
    token: Option<String>,
    sharetoken: Option<String>,
    key: Option<String>
}




pub async fn mw_must_be_admin(user: ConnectedUser, req: Request, next: Next) -> Result<Response> {

    match user {
        ConnectedUser::Server(user) => if user.role != UserRole::Admin {
            return Err(Error::Forbiden)
        },
        ConnectedUser::Anonymous | ConnectedUser::Guest(_) | ConnectedUser::UploadKey(_) => return Err(Error::Forbiden),
        ConnectedUser::ServerAdmin => {},
        ConnectedUser::Share(share) => {
            if share.kind != ClaimsLocalType::Admin {
                return Err(Error::Forbiden)
            }
        },
    }
    Ok(next.run(req).await)
}



pub async fn mw_token_resolver(mc: State<ModelController>, headers: HeaderMap, query: Query<TokenParams>, mut req: Request, next: Next) -> Result<Response> {
    let token: Option<String> = match headers.get("AUTHORIZATION").and_then(|t| t.to_str().ok()) {
        Some(token) => Some(token.replace(BEARER, "")),
        None => match &query.token {
            Some(token) => Some(token.clone()),
            None => None,
        },
    };
    let sharetoken: Option<String> = match headers.get("SHARETOKEN").and_then(|t| t.to_str().ok()) {
        Some(token) => Some(token.replace(BEARER, "")),
        None => match &query.sharetoken {
            Some(token) => Some(token.clone()),
            None => None,
        },
    };
    let auth_message = AuthMessage { token, sharetoken, key: query.key.clone()};
    let connected_user = parse_auth_message(&auth_message, &mc.0).await?;
    req.extensions_mut().insert(connected_user);

    Ok(next.run(req).await)
}

pub async fn parse_auth_message(auth: &AuthMessage, mc: &ModelController) -> Result<ConnectedUser> {
    let server_id =  get_server_id().await;

    if let Some(token) = &auth.token {
        if let Some(server_id) = server_id {  
            let claims = verify(&token, &server_id)?;
            let user = mc.get_user_unchecked(&claims.sub).await;
            match user {
                Ok(user) => Ok(ConnectedUser::Server(user)),
                Err(crate::model::error::Error::UserNotFound(_id)) => Ok(ConnectedUser::Guest(GuestUser { id: claims.sub.clone(), name: claims.name.clone() })),
                Err(err) => Err(err.into())
            }
        } else {
            Ok(ConnectedUser::Anonymous)
        }

    } else if let Some(sharetoken) = &auth.sharetoken {
        let claims = verify_local(&sharetoken).await?;
        
        Ok(ConnectedUser::Share(claims))

    } else if let Some(key) = &auth.key {
        let uploadkey = mc.get_upload_key(key.to_owned()).await?;
          
        Ok(ConnectedUser::UploadKey(uploadkey))
  
    } else {
        Ok(ConnectedUser::Anonymous)
    }
}


#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ConnectedUser {
	type Rejection = Error;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self> {

		let server_user = parts
			.extensions
			.get::<ConnectedUser>().ok_or(Error::AuthFail)?
            .clone();

        return  Ok(server_user);
    }
}
