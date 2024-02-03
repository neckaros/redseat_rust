#![allow(dead_code)]

use std::{net::{IpAddr, Ipv6Addr, SocketAddr}, path::PathBuf};

use axum::{
    http::Method,
    middleware,
    Router
};
use axum_server::tls_rustls::RustlsConfig;
use model::ModelController;
use routes::mw_auth;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::cors::{CorsLayer, Any};
use crate::{server::{get_config, update_ip}, tools::auth::get_or_init_keys};

pub use self::error::{Result, Error};

mod model;
mod routes;
mod error;
mod tools;
mod server;
mod certificate;



#[tokio::main]
async fn main() ->  Result<()> {
    
    println!("Starting redseat server");
    println!("Initializing config");
    server::initialize_config().await;

    

    let register_infos = register().await?;
    let app = app();
    if let Some(certs) = register_infos.cert_paths {
        println!("Starting HTTP/HTTPS server");
        println!("{:?}", certs);
        let tls_config = RustlsConfig::from_pem_chain_file(certs.0, certs.1).await.unwrap();

        let addr = SocketAddr::new(IpAddr::from(Ipv6Addr::UNSPECIFIED), 6969);

        println!("->> LISTENING HTTP/HTTPS on {:?}\n", addr);
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
    let mc = ModelController::new().await?;

    let cors: CorsLayer = CorsLayer::new()
    // allow `GET` and `POST` when accessing the resource
    .allow_methods(vec![Method::GET, Method::PATCH, Method::DELETE, Method::POST])
    // allow requests from any origin
    .allow_origin(Any);


    Ok(Router::new()
        .nest("/ping", routes::ping::routes())
        .nest("/libraries", routes::libraries::routes(mc.clone()))
        .nest("/users", routes::users::routes(mc.clone()))
        //.layer(middleware::map_response(main_response_mapper))
        .layer(middleware::from_fn_with_state(mc.clone(), mw_auth::mw_token_resolver))
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
    println!("Checking registration");
    let config = get_config().await;
    println!("Server ID: {}", config.id);   
    let _ = get_or_init_keys().await;
    
    let domain = config.domain.clone();
    let duck_dns = config.duck_dns.clone();
    let mut register_info = RegisterInfo {cert_paths: None, ips: None};
    let ips = update_ip().await.unwrap_or(None);
    register_info.ips = ips;
    

    if domain.is_some() && duck_dns.is_some() {
        println!("Public domain certificate check");

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
    use http_body_util::BodyExt; // for `collect`
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