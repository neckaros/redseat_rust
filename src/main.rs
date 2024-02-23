#![allow(dead_code)]

use std::{net::{IpAddr, Ipv6Addr, SocketAddr}, path::PathBuf};

use axum::{
    handler::Handler, http::Method, middleware, Extension, Router
};
use axum_server::tls_rustls::RustlsConfig;
use image::ImageOutputFormat;
use model::{store::SqliteStore, ModelController};
use routes::mw_auth::{self, parse_auth_message};


use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::cors::{CorsLayer, Any};
use crate::{server::{get_config, update_ip}, tools::{auth::get_or_init_keys, image_tools::resize_image_path, log::log_info}};
use socketioxide::{
    extract::{AckSender, Bin, TryData, Data, SocketRef},
    SocketIo,
};
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

    tokio::spawn(async move {
        resize_image_path("test_data/image.jpg", "test_data/image-thumb.jpg", 500, ImageOutputFormat::Jpeg(80)).await.unwrap()
        //tools::video_tools::convert_to_pipe("C:/Users/arnau/Downloads/IMG_5020.mov", None).await;
    });

    let register_infos = register().await?;
    let app = app();
    if let Some(certs) = register_infos.cert_paths {

        log_info(tools::log::LogServiceType::Register, format!("Starting HTTP/HTTPS server"));
        
        let tls_config = RustlsConfig::from_pem_chain_file(certs.0, certs.1).await.unwrap();

        let addr = SocketAddr::new(IpAddr::from(Ipv6Addr::UNSPECIFIED), 6969);
        log_info(tools::log::LogServiceType::Register, format!("->> LISTENING HTTP/HTTPS on {:?}\n", addr));

        axum_server_dual_protocol::bind_dual_protocol(addr, tls_config)
	.serve(app.await?.into_make_service())
	.await.unwrap();
        

    } else {
        let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();
        println!("->> LISTENING on {:?}\n", listener.local_addr());
        axum::serve(listener, app.await?)
            .await
            .unwrap();
    }

    



	// endregion: --- Start Server

	Ok(())
}



async fn app() -> Result<Router> {
    let store = SqliteStore::new().await.unwrap();
    let mut mc = ModelController::new(store).await?;

    let cors: CorsLayer = CorsLayer::new()
    // allow `GET` and `POST` when accessing the resource
    .allow_methods(vec![Method::GET, Method::PATCH, Method::DELETE, Method::POST])
    // allow requests from any origin
    .allow_origin(Any);
    let (iolayer, io) = SocketIo::builder().with_state(mc.clone()).build_layer();
    io.ns("/", routes::socket::on_connect);
    mc.set_socket(io.clone());

    Ok(Router::new()
        .nest("/ping", routes::ping::routes())
        .nest("/libraries", routes::libraries::routes(mc.clone()))
        .nest("/library", routes::libraries::routes(mc.clone())) // duplicate for legacy
        .nest("/users", routes::users::routes(mc.clone()))
        //.layer(middleware::map_response(main_response_mapper))
        .layer(middleware::from_fn_with_state(mc.clone(), mw_auth::mw_token_resolver))
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
    
    let domain = config.domain.clone();
    let duck_dns = config.duck_dns.clone();
    let mut register_info = RegisterInfo {cert_paths: None, ips: None};
    let ips = update_ip().await.unwrap_or(None);
    register_info.ips = ips;
    

    if domain.is_some() && duck_dns.is_some() {
        log_info(tools::log::LogServiceType::Register, "Public domain certificate check".to_string());

        let certs = certificate::dns_certify(&domain.unwrap(), &duck_dns.unwrap()).await?;
        register_info.cert_paths = Some(certs);
    } 


    Ok(register_info)
    //println!("DuckDns domain not found");

    //println!("Please enter your duckdns domain and press enter:");
    //let mut input_string = String::new();
    //io::stdin().read_line(&mut input_string).unwrap();
    //println!("You wrote {:?}", input_string.trim());
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