use crate::{model::{users::{ConnectedUser, HistoryQuery}, ModelController}, Result};
use axum::{extract::{Path, State}, middleware, routing::get, Json, Router};
use axum_extra::extract::Query;
use serde_json::{json, Value};

use super::mw_auth;



pub fn routes(mc: ModelController) -> Router {

	let admin_routes = 	Router::new()
		.route("/", get(handler_list))
		.route_layer(middleware::from_fn_with_state(mc.clone(), mw_auth::mw_must_be_admin))
		.with_state(mc.clone());


	Router::new()
		.route("/me", get(handler_me))
		.route("/:id", get(handler_id))
		.route("/me/history", get(handler_list_history))
		.merge(admin_routes)
		.with_state(mc)
	
        
}

async fn handler_me(user: ConnectedUser) -> Result<Json<Value>> {
	let body = Json(json!(user));
	Ok(body)
}

async fn handler_id(Path(user_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let user = mc.get_user(&user_id, &user).await?;
	let body = Json(json!(user));
	Ok(body)
}


async fn handler_list(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let users = mc.get_users(&user).await?;
	let body = Json(json!(users));

	Ok(body)
}

async fn handler_list_history(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<HistoryQuery>) -> Result<Json<Value>> {
	let users = mc.get_watched(query, &user).await?;
	let body = Json(json!(users));

	Ok(body)
}