use axum::async_trait;
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};

use crate::model::users::{ConnectedUser, UserRole};

use crate::{error::Error, Result};

const BEARER: &str = "Bearer ";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RangeParam {
    range: Option<String>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RangeDefinition {
    pub start: Option<u64>,
    pub end: Option<u64>
}



pub async fn mw_range(headers: HeaderMap, query: Query<RangeParam>, mut req: Request, next: Next) -> Result<Response> {
    let range: Option<String> = match headers.get("range").and_then(|t| t.to_str().ok()) {
        Some(range) => Some(range.to_string()),
        None => match &query.range {
            Some(range) => Some(range.to_string()),
            None => None,
        },
    };
    
    if let Some(range) = range {
        let range_definition = parse_range(range)?;
        req.extensions_mut().insert(range_definition);
    }
    Ok(next.run(req).await)
}

pub fn parse_range(range: String) -> Result<RangeDefinition> {
    if !range.contains("bytes=") {
        return Err(Error::InvalidRangeHeader);
    }
    let range: Vec<Option<u64>> = range.replace("bytes=", "").split("-").map(|e| e.parse::<u64>().ok() ).collect();
    Ok(RangeDefinition { start: range.get(0).unwrap_or(&None).clone(), end: range.get(1).unwrap_or(&None).clone() })
}


#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for RangeDefinition {
	type Rejection = Error;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self> {

		let range = parts
			.extensions
			.get::<RangeDefinition>().ok_or(Error::NoRangeHeader)?
            .clone();

        return  Ok(range);
    }
}
