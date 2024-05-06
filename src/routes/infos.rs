use crate::{model::{users::ConnectedUser, ModelController}, server::get_config, Result};
use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_infos))
		.with_state(mc)
}

async fn handler_infos(State(mc): State<ModelController>) -> Result<Json<Value>> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	let config = get_config().await;
	let body = Json(json!({
		"id": config.id,
		"administred": admin_users.len() > 0,
		"domain": config.domain,
		"port": config.port,
		"local": config.local,
	}));

	Ok(body)
}
