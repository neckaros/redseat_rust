use crate::{model::ServerUser, Result};
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};



pub fn routes() -> Router {
	Router::new().route("/", get(handler_libraries))
        
}

async fn handler_libraries(test: ServerUser) -> Result<Json<Value>> {
    println!("FROM R{:?}", test);
	let body = Json(json!({
		"result": {
			"success": true,
			"username": test.user_id()
		}
	}));

	Ok(body)
}
