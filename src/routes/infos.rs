use crate::{model::{users::{ConnectedUser, ServerUser, UserRole}, ModelController}, server::get_config, tools::image_tools::image_magick::Red, Result};
use axum::{extract::{Query, State}, response::Redirect, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_infos))
		.route("/install", get(handler_install))
		.route("/own", get(handler_own))
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

async fn handler_install(State(mc): State<ModelController>) -> Result<Redirect> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	let config = get_config().await;
	let body = Json(json!({
		"id": config.id,
		"administred": admin_users.len() > 0,
		"domain": config.domain,
		"port": config.port,
		"local": config.local,
	}));

	let mut params = vec![];
	if let Some(domain) = config.domain {
		params.push(format!("domain={}", domain));
	}
	if let Some(port) = config.port {
		params.push(format!("port={}", port));
	}
	if let Some(local) = config.local {
		params.push(format!("local={}", local));
	}
	params.push(format!("administred={}", admin_users.len() > 0));
	
	if admin_users.len() > 0 {
		Ok(Redirect::temporary(&format!("https://{}/servers/{}/settings", config.redseat_home, config.id)))
	} else {
		Ok(Redirect::temporary(&format!("https://{}/install/{}?{}", config.redseat_home, config.id, params.join("&"))))
	}
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OwnQuery {
    user: String,
	name: String,
}


async fn handler_own(State(mc): State<ModelController>, Query(query): Query<OwnQuery>) -> Result<Redirect> {
	let admin_users = mc.get_users(&ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
	let config = get_config().await;

	if admin_users.len() > 0 {
		Ok(Redirect::temporary(&format!("https://{}/servers/{}/settings", config.redseat_home, config.id)))
	} else {
		let user = ServerUser {
			id: query.user,
			name: query.name,
			role: UserRole::Admin,
			..Default::default()
		};
		mc.add_user(user, &ConnectedUser::ServerAdmin).await?;
		Ok(Redirect::temporary(&format!("https://{}/servers/{}/settings", config.redseat_home, config.id)))
	}
}