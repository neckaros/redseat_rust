#![cfg_attr(debug_assertions, allow(dead_code, unused_imports))]
#![allow(warnings)]

use std::{fs, net::{IpAddr, Ipv6Addr, SocketAddr}, path::PathBuf, str::FromStr, time::{SystemTime, UNIX_EPOCH}};

use axum::{
    extract::DefaultBodyLimit, http::Method, middleware, serve, Router
};
use axum_server::tls_rustls::RustlsConfig;

use domain::ffmpeg;
use error::RsError;
use http::{StatusCode, Uri};
use hyper::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, REFERRER_POLICY, REFERER};
use model::{server::AuthMessage, store::SqliteStore, ModelController};
use plugins::{medias::{imdb::ImdbContext, tmdb::{tmdb_configuration::TmdbConfiguration, TmdbContext}, trakt::TraktContext}, PluginManager};
use routes::{mw_auth, mw_range};


use server::{get_home, get_server_id, get_server_port, PublicServerInfos};
use tokio::net::TcpListener;
use tools::{auth::{sign_local, Claims}, image_tools::has_image_magick, log::{log_error, LogServiceType}, prediction, video_tools::VideoCommandBuilder};
use tower::ServiceBuilder;
use tower_http::{cors::{Any, CorsLayer}, trace::TraceLayer};
use crate::{server::{get_config, update_ip}, tools::{auth::{get_or_init_keys, verify_local, ClaimsLocal}, image_tools::resize_image_path, log::log_info}};
use socketioxide::{extract::{SocketRef, TryData}, SocketIo};
pub use self::error::{Result, Error};


use tracing_subscriber::fmt::fmt;
use tracing_subscriber::filter::EnvFilter;

mod model;
mod routes;
mod error;
mod tools;
mod server;
mod certificate;
mod plugins;
mod domain;


#[tokio::main(flavor = "multi_thread")]
async fn main() ->  Result<()> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    log_info(tools::log::LogServiceType::Register, format!("Architecture: {}-{}", os, arch));
    log_info(tools::log::LogServiceType::Register, "Starting redseat server".to_string());
    log_info(tools::log::LogServiceType::Register, "Initializing config".to_string());

    let ffmpeg_version = VideoCommandBuilder::version().await?;
    if let Some(ffmpeg_version) = ffmpeg_version {
        log_info(tools::log::LogServiceType::Register, format!("FFMPEG version {:?}", ffmpeg_version));
    } else {
        log_info(tools::log::LogServiceType::Register, "No FFMPEG found, downloading latest version in background. Video operations won't be available in the meantime".to_string());
        tokio::spawn(async {
            VideoCommandBuilder::download().await?;
            let ffmpeg_version = VideoCommandBuilder::version().await?;
            if let Some(ffmpeg_version) = ffmpeg_version {
                log_info(tools::log::LogServiceType::Register, format!("FFMPEG version {:?}", ffmpeg_version));
            } else {
                log_error(tools::log::LogServiceType::Register, "We were not able to confirm FFMPEG installation".to_string());
            }
            Ok::<(), RsError>(())
        });
    }
    

    let config = server::initialize_config().await;

    if !config.imagesUseIm {
        log_info(tools::log::LogServiceType::Register, "Will use native libraries for image conversions".to_string());
    } else {
        log_info(tools::log::LogServiceType::Register, "Will use ImageMagick for image conversions".to_string());
    }


    let register_infos = register().await?;
    let app = app();
    let local_port = get_server_port().await;
    if let Some(certs) = register_infos.cert_paths {

        log_info(tools::log::LogServiceType::Register, format!("Starting HTTP/HTTPS server"));
        
        let tls_config = RustlsConfig::from_pem_chain_file(certs.0, certs.1).await.unwrap();

        //let addr = format!("[::]:{}", local_port).parse::<SocketAddr>().unwrap();
        let addr = SocketAddr::from(([0, 0, 0, 0], local_port));
        log_info(tools::log::LogServiceType::Register, format!("->> LISTENING HTTP/HTTPS on {:?}\n", addr));

        axum_server_dual_protocol::bind_dual_protocol(addr, tls_config)
            .serve(app.await?.into_make_service())
            .await.unwrap();
        

    } else {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", local_port)).await.unwrap();
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

    let origins = [
        "http://localhost:3000".parse().unwrap(),
        "https://www.redseat.cloud".parse().unwrap(),
    ];

    let cors: CorsLayer = CorsLayer::new()
    // allow `GET` and `POST` when accessing the resource
    .allow_methods(vec![Method::GET, Method::PATCH, Method::DELETE, Method::HEAD, Method::OPTIONS, Method::POST])
    .allow_headers([AUTHORIZATION, ACCEPT, CONTENT_TYPE,REFERRER_POLICY,REFERER])
    // allow requests from any origin

    .allow_origin(origins);
    let (iolayer, io) = SocketIo::builder().with_state(mc.clone()).build_layer();
    //io.ns("/", routes::socket::on_connect);
    mc.set_socket(io.clone());

    let mc_forsocket = mc.clone();
    io.ns("/", {
        |socket: SocketRef, TryData(data): TryData<AuthMessage>| async move { routes::socket::on_connect(socket, mc_forsocket, data).await }
      });

    let server_id = get_server_id().await;
    let admin_users = mc.get_users(&model::users::ConnectedUser::ServerAdmin).await?.into_iter().filter(|u| u.is_admin()).collect::<Vec<_>>();
    if admin_users.is_empty() || server_id.is_none() {
        log_info(LogServiceType::Register, format!("Register your server at: http://127.0.0.1:{}/infos/install", get_server_port().await));
    }
    Ok(Router::new()
        .nest("/ping", routes::ping::routes())
        .nest("/infos", routes::infos::routes(mc.clone()))
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
        .fallback(fallback)
        .layer(middleware::from_fn(mw_range::mw_range))
        //.layer(middleware::map_response(main_response_mapper))
        .layer(middleware::from_fn_with_state(mc.clone(), mw_auth::mw_token_resolver))
        .layer(DefaultBodyLimit::disable())
        .layer(
            ServiceBuilder::new()
                .layer(iolayer),
        )
        .layer(
        ServiceBuilder::new()
            .layer(cors)
            
        )
        .layer(TraceLayer::new_for_http())
    )
        

}
async fn fallback(uri: Uri) -> (StatusCode, &'static str) {
    log_info(LogServiceType::Other, format!("Route not found: {}", uri));
    (StatusCode::NOT_FOUND, "Not Found")
}
struct RegisterInfo {
    cert_paths: Option<(PathBuf, PathBuf)>,
    ips: Option<String>
}

async fn register() -> Result<RegisterInfo>{
    log_info(tools::log::LogServiceType::Register, "Checking registration".to_string());
    let config = get_config().await;
    if let Some(id) = config.id.clone() {
        log_info(tools::log::LogServiceType::Register, format!("Server ID: {}", id));   
    }
    let _ = get_or_init_keys().await;

    let mut register_info = RegisterInfo {cert_paths: None, ips: None};
    
    if let (Some(id), Some(_)) = (config.id, config.token) {
        let ips = update_ip().await?;
        register_info.ips = ips;

        log_info(tools::log::LogServiceType::Register, "Public domain certificate check".to_string());
        let certs = certificate::dns_certify().await?;
        register_info.cert_paths = Some(certs.clone());

        

        let public_config = PublicServerInfos::get(&certs.0, &id).await?;
        log_info(LogServiceType::Register, format!("Exposed public url: {}:{}", id, public_config.port));
    } 

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