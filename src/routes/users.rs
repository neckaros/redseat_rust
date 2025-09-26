use crate::{domain::{view_progress::ViewProgressForAdd, watched::{Watched, WatchedForAdd}}, model::{users::{ConnectedUser, HistoryQuery, InvitationRedeemer, UserRole, ViewProgressQuery}, ModelController}, Result};
use axum::{extract::{Path, State}, middleware, routing::{get, post}, Json, Router};
use axum_extra::extract::Query;
use serde_json::{json, Value};
use tower_http::trace::TraceLayer;

use super::mw_auth;



pub fn routes(mc: ModelController) -> Router {

	let admin_routes = 	Router::new()
		.route("/", get(handler_list))
		.route_layer(middleware::from_fn_with_state(mc.clone(), mw_auth::mw_must_be_admin))
		.with_state(mc.clone());


	Router::new()
		.route("/me", get(handler_me))
		.route("/:id", get(handler_id))
		.route("/admin/history", get(handler_list_history_all))
		.route("/admin/history/import", post(handler_add_all_history))
		.route("/me/history", get(handler_list_history))
		.route("/me/history", post(handler_add_history))
		.route("/me/history/progress/:id", get(handler_get_progress))
		.route("/me/history/progress", post(handler_add_progress))

		.route("/invitation", get(handler_invitation))
		.merge(admin_routes)
		
        .layer(TraceLayer::new_for_http())
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
	let users = mc.get_watched(query, &user, None).await?;
	let body = Json(json!(users));

	Ok(body)
}

async fn handler_list_history_all(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let users = mc.get_all_watched(&user).await?;
	let body = Json(json!(users));

	Ok(body)
}

async fn handler_add_all_history(State(mc): State<ModelController>, user: ConnectedUser, Json(watcheds): Json<Vec<WatchedForAdd>>) -> Result<Json<Value>> {
	user.check_role(&UserRole::Admin)?;
	for watched in watcheds {
		mc.add_watched(watched, &user, None).await?;
	}
	let body = Json(json!({"ok": true}));

	Ok(body)
}

async fn handler_add_history(State(mc): State<ModelController>, user: ConnectedUser, Json(watched): Json<WatchedForAdd>) -> Result<Json<Value>> {
	mc.add_watched(watched, &user, None).await?;
	let body = Json(json!({"ok": true}));

	Ok(body)
}

async fn handler_get_progress(Path(id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let progress = mc.get_view_progress_by_id( id, &user).await?;
	let body = Json(json!(progress));

	Ok(body)
}

async fn handler_add_progress(State(mc): State<ModelController>, user: ConnectedUser, Json(watched): Json<ViewProgressForAdd>) -> Result<Json<Value>> {
	mc.add_view_progress(watched, &user, None).await?;
	let body = Json(json!({"ok": true}));

	Ok(body)
}



async fn handler_invitation(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<InvitationRedeemer>) -> Result<Json<Value>> {
	let library =  mc.redeem_invitation(query.code, user.clone()).await?;

	let body = Json(json!(json!({"library": library, "uid": user.user_id()?, "name": user.user_name()?, "user": user})));
	Ok(body)
}
