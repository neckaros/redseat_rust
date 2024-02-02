use crate::{model::{ModelController, users::ServerUser}, Result};
use axum::{extract::State, middleware, routing::get, Json, Router};
use serde_json::{json, Value};

use super::mw_auth;



pub fn routes(mc: ModelController) -> Router {

	let admin_routes = 	Router::new()
		.route("/", get(handler_list))
		.route_layer(middleware::from_fn_with_state(mc.clone(), mw_auth::mw_must_be_admin))
		.with_state(mc.clone());


	Router::new()
		.route("/me", get(handler_me))
		.merge(admin_routes)
		.with_state(mc)
	
        
}

async fn handler_me(user: ServerUser) -> Result<Json<Value>> {
	let body = Json(json!({
		"result": {
			"success": true,
			"user": user,
		}
	}));
	Ok(body)
}

async fn handler_list(State(mc): State<ModelController>, user: ServerUser) -> Result<Json<Value>> {
	let users = mc.get_users().await?;
	let body = Json(json!({
		"result": {
			"success": true,
			"users": users,
		}
	}));

	Ok(body)
}
