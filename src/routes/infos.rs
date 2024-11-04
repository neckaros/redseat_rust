use std::time::Duration;

use crate::{model::{users::{ConnectedUser, ServerUser, UserRole}, ModelController}, server::{check_unregistered, get_config, get_install_url, get_own_url, get_server_file_path, get_server_id, get_web_url, update_config, PublicServerInfos}, tools::{image_tools::image_magick::Red, log::{log_info, LogServiceType}}, Result};
use axum::{extract::{Query, State}, response::Redirect, routing::{get, post}, Json, Router};
use query_external_ip::Consensus;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::{sleep, Sleep};


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_infos))
		.route("/install", get(handler_install))
		.route("/configure", get(handler_configure))
		.route("/register", get(handler_register))
		.with_state(mc)
}

async fn handler_infos(State(mc): State<ModelController>) -> Result<Json<Value>> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	let infos = PublicServerInfos::current().await?;
	let body: Json<Value> = Json(json!({
		"administred": !admin_users.is_empty(),
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

async fn handler_own(State(mc): State<ModelController>, Query(query): Query<OwnQuery>) -> Result<Redirect> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	if !admin_users.is_empty() {
		Err(crate::Error::ServerAlreadyOwned)
	} else {
		let user = ServerUser {
			id: query.user,
			name: query.name,
			role: UserRole::Admin,
			..Default::default()
		};
		mc.add_user(user, &ConnectedUser::ServerAdmin).await?;
		let url = get_install_url().await?;
		Ok(Redirect::temporary(&url))
	}
}

async fn handler_install() -> Result<Redirect> {
	log_info(LogServiceType::Register, "Getting install request checking that server is unregistered".to_string());
	//let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	check_unregistered().await?;
    
    
	let url = get_install_url().await?;
	
	log_info(LogServiceType::Register, format!("Install URL: {}", url));
	Ok(Redirect::temporary(&url))
	
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigureQuery {
    domain: String,
	duckdns: Option<String>,
	port: Option<u16>,
}

async fn handler_configure(Query(query): Query<ConfigureQuery>) -> Result<Redirect> {
	check_unregistered().await?;

	let mut config = get_config().await;

	config.port = query.port;

	update_config(config).await?;
	let url = get_install_url().await?;

	tokio::spawn(async move {
		sleep(Duration::from_millis(500)).await;
		std::process::exit(201);
	});
	Ok(Redirect::temporary(&url))

}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisterQuery {
    id: String,
	token: String,
	uid: String,
	username: String,
}

async fn handler_register(State(mc): State<ModelController>, Query(query): Query<RegisterQuery>) -> Result<Redirect> {
	check_unregistered().await?;

	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	if !admin_users.is_empty() {
		Err(crate::Error::ServerAlreadyOwned)
	} else {
		let user = ServerUser {
			id: query.uid,
			name: query.username,
			role: UserRole::Admin,
			..Default::default()
		};
		mc.add_user(user, &ConnectedUser::ServerAdmin).await?;

		let mut config = get_config().await;

		config.id = Some(query.id.clone());
		config.token = Some(query.token);
		let home = config.redseat_home.clone();
		update_config(config).await?;
		let server_id = get_server_id().await.ok_or(crate::Error::Error("Failed to set ID".to_string()));


		let mut params = vec![];
		params.push(format!("id={}", query.id));
		
		
		let url = format!("https://{}/install/final?id={}", home, params.join("&"));

		tokio::spawn(async move {
			sleep(Duration::from_millis(500)).await;
			std::process::exit(201);
		});
		

		Ok(Redirect::temporary(&url))
	}

}