use crate::{model::users::ServerUser, Result};
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};



pub fn routes() -> Router {
	Router::new().route("/", get(handler_libraries))
        
}

async fn handler_libraries(user: ServerUser) -> Result<Json<Value>> {
	let body = Json(json!({
		"result": {
			"success": true,
			"user": user,
		}
	}));

	Ok(body)
}
