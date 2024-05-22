use std::time::Duration;

use crate::{model::{users::{ConnectedUser, ServerUser, UserRole}, ModelController}, server::{check_unregistered, get_config, get_install_url, get_own_url, get_server_file_path, get_server_id, get_web_url, update_config, PublicServerInfos}, tools::image_tools::image_magick::Red, Result};
use axum::{extract::{Query, State}, response::Redirect, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::{sleep, Sleep};


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_infos))
		.route("/install", get(handler_install))
		.route("/own", get(handler_own))
		.route("/configure", get(handler_configure))
		.route("/register", post(handler_register))
		.with_state(mc)
}

async fn handler_infos(State(mc): State<ModelController>) -> Result<Json<Value>> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	let infos = PublicServerInfos::current().await?;
	let body: Json<Value> = Json(json!({
		"administred": admin_users.len() > 0,
		"publicInfos": infos,
	}));
	Ok(body)
}

async fn handler_restart(user: ConnectedUser) -> Result<Json<Value>> {
	user.check_role(&UserRole::Admin)?;
	std::process::exit(201);
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OwnQuery {
    user: String,
	name: String,
}

async fn handler_own(State(mc): State<ModelController>, Query(query): Query<OwnQuery>,  user: ConnectedUser) -> Result<Redirect> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	if admin_users.len() > 0 {
		Err(crate::Error::ServerAlreadyOwned)
	} else {
		let user = ServerUser {
			id: query.user,
			name: query.name,
			role: UserRole::Admin,
			..Default::default()
		};
		mc.add_user(user, &ConnectedUser::ServerAdmin).await?;
		let url = get_install_url(&mc).await?;
		Ok(Redirect::temporary(&url))
	}
}

async fn handler_install(State(mc): State<ModelController>,  user: ConnectedUser) -> Result<Redirect> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	check_unregistered().await?;
	if admin_users.len() > 0 {
		let url = get_install_url(&mc).await?;
		Ok(Redirect::temporary(&url))
	} else {
		let url = get_own_url().await?;
		Ok(Redirect::temporary(&url))
	}
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigureQuery {
    domain: String,
	duckdns: Option<String>,
	port: Option<u16>,
}

async fn handler_configure(State(mc): State<ModelController>, Query(query): Query<ConfigureQuery>,  user: ConnectedUser) -> Result<Redirect> {
	check_unregistered().await?;

	let mut config = get_config().await;

	config.port = query.port;
	config.domain = Some(query.domain);
	config.duck_dns = query.duckdns;

	update_config(config).await?;
	let url = get_install_url(&mc).await?;

	tokio::spawn(async move {
		sleep(Duration::from_millis(500)).await;
		std::process::exit(201);
	});
	Ok(Redirect::temporary(&url))

}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisterQuery {
    id: String,
}

async fn handler_register(user: ConnectedUser, Json(query): Json<RegisterQuery>) -> Result<Json<Value>> {
	check_unregistered().await?;

	let mut config = get_config().await;

	config.id = Some(query.id);
	update_config(config).await?;
	let server_id = get_server_id().await.ok_or(crate::Error::Error("Failed to set ID".to_string()));
	Ok(Json(json!({
		"id": server_id,
	})))

}