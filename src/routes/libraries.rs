use crate::{model::{users::ConnectedUser, ModelController}, Result};
use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};



pub fn routes(mc: ModelController) -> Router {
	Router::new().route("/", get(handler_libraries))
	.with_state(mc)
        
}

async fn handler_libraries(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_libraries(&user).await?;
	let body = Json(json!({
		"result": {
			"success": true,
			"libraries": libraries,
		}
	}));

	Ok(body)
}
