#![cfg_attr(debug_assertions, allow(dead_code, unused_imports))]

use std::{fs, net::{IpAddr, Ipv6Addr, SocketAddr}, path::PathBuf, str::FromStr, time::{SystemTime, UNIX_EPOCH}};

use axum::{
    extract::DefaultBodyLimit, http::Method, middleware, Router
};
use axum_server::tls_rustls::RustlsConfig;

use domain::MediasIds;
use hyper::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use model::{store::SqliteStore, ModelController};
use plugins::{medias::{imdb::ImdbContext, tmdb::{tmdb_configuration::TmdbConfiguration, TmdbContext}, trakt::TraktContext}, PluginManager};
use routes::{mw_auth, mw_range};


use server::{get_home, get_server_port, PublicServerInfos};
use tokio::net::TcpListener;
use tools::{auth::{sign_local, Claims}, log::LogServiceType, prediction};
use tower::ServiceBuilder;
use tower_http::cors::{CorsLayer, Any};
use crate::{server::{get_config, update_ip}, tools::{auth::{get_or_init_keys, verify_local, ClaimsLocal}, image_tools::resize_image_path, log::log_info}};
use socketioxide::SocketIo;
pub use self::error::{Result, Error};

mod model;
mod routes;
mod error;
mod tools;
mod server;
mod certificate;
mod plugins;
mod domain;


#[tokio::main]
async fn main() ->  Result<()> {
    log_info(tools::log::LogServiceType::Register, format!("Starting redseat server"));
    log_info(tools::log::LogServiceType::Register, format!("Initializing config"));
    server::initialize_config().await;

    /*tokio::spawn(async move {
        //let tmdb = TmdbContext::new("4a01db3a73eed5cf17e9c7c27fd9d008".to_string()).await.unwrap();
        //tmdb.serie_image(MediasIds::from_tmdb(236235)).await.unwrap();
        //trakt.get_serie(&MediasIds { imdb: Some("tt0944947".to_string()), ..Default::default()}).await;
        //trakt.all_episodes(&MediasIds { imdb: Some("tt0944947".to_string()), ..Default::default()}).await;

    });*/

    let register_infos = register().await?;
    let app = app();
    let local_port = get_server_port().await;
    if let Some(certs) = register_infos.cert_paths {

        log_info(tools::log::LogServiceType::Register, format!("Starting HTTP/HTTPS server"));
        
        let tls_config = RustlsConfig::from_pem_chain_file(certs.0, certs.1).await.unwrap();

        let addr = SocketAddr::new(IpAddr::from(Ipv6Addr::UNSPECIFIED), local_port);
        log_info(tools::log::LogServiceType::Register, format!("->> LISTENING HTTP/HTTPS on {:?}\n", addr));

        axum_server_dual_protocol::bind_dual_protocol(addr, tls_config)
            .serve(app.await?.into_make_service())
            .await.unwrap();
        

    } else {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", local_port)).await.unwrap();
        log_info(LogServiceType::Register, format!("->> LISTENING on {:?}\n", listener.local_addr()));
        
        axum::serve(listener, app.await?)
            .await
            .unwrap();
    }

    



	// endregion: --- Start Server

	Ok(())
}



async fn app() -> Result<Router> {
    let store = SqliteStore::new().await.unwrap();
    let plugin_manager = PluginManager::new().await?;
    let mut mc = ModelController::new(store, plugin_manager).await?;

    let cors: CorsLayer = CorsLayer::new()
    // allow `GET` and `POST` when accessing the resource
    .allow_methods(vec![Method::GET, Method::PATCH, Method::DELETE, Method::HEAD, Method::OPTIONS, Method::POST])
    .allow_headers([AUTHORIZATION, ACCEPT, CONTENT_TYPE])
    // allow requests from any origin
    .allow_origin(Any);
    let (iolayer, io) = SocketIo::builder().with_state(mc.clone()).build_layer();
    io.ns("/", routes::socket::on_connect);
    mc.set_socket(io.clone());

    Ok(Router::new()
        .nest("/ping", routes::ping::routes())
        .nest("/libraries", routes::libraries::routes(mc.clone()))
        .nest("/libraries/:libraryid/medias", routes::medias::routes(mc.clone()))
        .nest("/libraries/:libraryid/tags", routes::tags::routes(mc.clone()))
        .nest("/libraries/:libraryid/people", routes::people::routes(mc.clone()))
        .nest("/libraries/:libraryid/series", routes::series::routes(mc.clone()))
        .nest("/libraries/:libraryid/movies", routes::movies::routes(mc.clone()))
        .nest("/libraries/:libraryid/plugins", routes::library_plugins::routes(mc.clone()))
        .nest("/library", routes::libraries::routes(mc.clone())) // duplicate for legacy
        .nest("/users", routes::users::routes(mc.clone()))
        .nest("/credentials", routes::credentials::routes(mc.clone()))
        .nest("/backups", routes::backups::routes(mc.clone()))
        .nest("/plugins", routes::plugins::routes(mc.clone()))
        .layer(middleware::from_fn(mw_range::mw_range))
        //.layer(middleware::map_response(main_response_mapper))
        .layer(middleware::from_fn_with_state(mc.clone(), mw_auth::mw_token_resolver))
        .layer(DefaultBodyLimit::disable())
        .layer(
            ServiceBuilder::new()
                .layer(CorsLayer::permissive()) // Enable CORS policy
                .layer(iolayer),
        )
        .layer(
        ServiceBuilder::new()
            .layer(cors)
            
        )
    )
        

}

struct RegisterInfo {
    cert_paths: Option<(PathBuf, PathBuf)>,
    ips: Option<(String, String)>
}

async fn register() -> Result<RegisterInfo>{
    log_info(tools::log::LogServiceType::Register, "Checking registration".to_string());
    let config = get_config().await;
    log_info(tools::log::LogServiceType::Register, format!("Server ID: {}", config.id));   
    let _ = get_or_init_keys().await;

    /* 
    let exp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let claims = ClaimsLocal {
        cr: "test".to_string(),
        kind: tools::auth::ClaimsLocalType::Admin,
        exp: exp.as_secs() + 60,
    };

    let token = sign_local(claims).await?;
    println!("TOJEN {}", token);

    let claims = verify_local(&token).await?;
    
    println!("verified {:?}", claims);
    */


    let domain = config.domain.clone();
    let duck_dns = config.duck_dns.clone();
    let mut register_info = RegisterInfo {cert_paths: None, ips: None};
    let ips = update_ip().await.unwrap_or(None);
    register_info.ips = ips;
    

    if domain.is_some() && duck_dns.is_some() {
        log_info(tools::log::LogServiceType::Register, "Public domain certificate check".to_string());
        let public_domain = domain.unwrap();
        let certs = certificate::dns_certify(&public_domain, &duck_dns.unwrap()).await?;
        register_info.cert_paths = Some(certs.clone());

        

        let public_config = PublicServerInfos::get(&certs.0, &public_domain).await?;
        log_info(LogServiceType::Register, format!("Exposed public url: {}:{}", public_config.url, public_config.port));
        
        let client = reqwest::Client::new();
        println!("server: {}", format!("https://{}/servers/{}/register", config.redseat_home, config.id));
        let res = client.post(format!("https://{}/servers/{}/register", config.redseat_home, config.id))
            .json(&public_config)
            .send()
            .await?;

        let register_response = res.text().await?;
        println!("repsonse: {}", register_response);
    } 

    //const register = await axios.post(`https://${process.env.REDSEAT_HOME}/servers/${serverId}/register`, publicInfo)

    Ok(register_info)

} 



#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{self, Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    // for `collect`
    use serde_json::{json, Value};
    use tower::ServiceExt; // for `call`, `oneshot`, and `ready`

    #[tokio::test]
    async fn json() {
        let app = app();

        let response = app.await.unwrap()
            .oneshot(
                Request::builder()
                    .method(http::Method::GET)
                    .uri("/ping")
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .body(Body::empty())
                    //.body(Body::from(
                    //    serde_json::to_vec(&json!([1, 2, 3, 4])).unwrap(),
                    //))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "*",
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body, json!({ "result": {"success": true} }));
    }

    #[tokio::test]
    async fn not_found() {
        let app = app();

        let response = app.await.unwrap()
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty());
    }


    
}