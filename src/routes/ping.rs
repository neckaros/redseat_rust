use crate::Result;
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};


pub fn routes() -> Router {
	Router::new().route("/", get(handler_ping))
}

async fn handler_ping() -> Result<Json<Value>> {
	let body = Json(json!({
		"result": {
			"success": true
		}
	}));

	Ok(body)
}
