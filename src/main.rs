#![cfg_attr(debug_assertions, allow(dead_code, unused_imports))]
#![allow(warnings)]

use std::{
    fs,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{extract::DefaultBodyLimit, http::Method, middleware, serve, Router};
use axum_server::tls_rustls::RustlsConfig;

use domain::ffmpeg;
use error::RsError;
use http::{StatusCode, Uri};
use hyper::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
};
use model::{server::AuthMessage, store::SqliteStore, ModelController};
use plugins::{
    medias::{imdb::ImdbContext, trakt::TraktContext},
    PluginManager,
};
use routes::{mw_auth, mw_range};

pub use self::error::{Error, Result};
use crate::{
    server::get_config,
    tools::{
        auth::{get_or_init_keys, verify_local, ClaimsLocal},
        log::log_info,
    },
};
use server::{get_home, get_server_id, get_server_port, PublicServerInfos};
use tokio::net::TcpListener;
use tools::{
    auth::{sign_local, Claims},
    image_tools::has_image_magick,
    log::{log_error, LogServiceType},
    prediction,
    video_tools::{ytdl::YydlContext, VideoCommandBuilder},
};
use tower::ServiceBuilder;
use tower_http::{
    cors::{AllowHeaders, Any, CorsLayer},
    trace::TraceLayer,
};

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::fmt;

mod certificate;
mod direct;
mod domain;
mod error;
mod model;
mod plugins;
mod routes;
mod server;
mod tools;
mod webrtc;

/// Target soft limit on open files; capped by the process hard limit.
const OPEN_FILES_LIMIT: u64 = 65_536;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    // Rustls 0.23+ requires an explicit crypto provider when both aws-lc-rs and ring are available.
    // Ring is used here for cross-platform compatibility (aws-lc-rs has build issues on Windows).
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    log_info(
        tools::log::LogServiceType::Register,
        format!("Architecture: {}-{}", os, arch),
    );

    // Containers commonly start with a 1024 soft limit on open files. Every
    // WebRTC peer holds several UDP sockets, so raise it toward the hard limit.
    match rlimit::increase_nofile_limit(OPEN_FILES_LIMIT) {
        Ok(limit) => log_info(
            LogServiceType::Register,
            format!("Open files limit: {limit}"),
        ),
        Err(error) => log_error(
            LogServiceType::Register,
            format!("Unable to raise the open files limit: {error}"),
        ),
    }

    log_info(
        tools::log::LogServiceType::Register,
        "Starting redseat server".to_string(),
    );
    log_info(
        tools::log::LogServiceType::Register,
        "Initializing config".to_string(),
    );

    extism::set_log_callback(
        |line| {
            println!("Extism Log: {}", line);
        },
        "info",
    )?;

    let ffmpeg_version = VideoCommandBuilder::version().await?;
    if let Some(ffmpeg_version) = ffmpeg_version {
        log_info(
            tools::log::LogServiceType::Register,
            format!("FFMPEG version {:?}", ffmpeg_version),
        );
    } else {
        log_info(tools::log::LogServiceType::Register, "No FFMPEG found, downloading latest version in background. Video operations won't be available in the meantime".to_string());
        tokio::spawn(async {
            let downloading = VideoCommandBuilder::download().await;

            if let Err(e) = downloading {
                log_error(
                    tools::log::LogServiceType::Register,
                    format!("We were not able to download FFMPEG: {}", e),
                );
                return Ok::<(), RsError>(());
            }

            let ffmpeg_version = VideoCommandBuilder::version().await?;
            if let Some(ffmpeg_version) = ffmpeg_version {
                log_info(
                    tools::log::LogServiceType::Register,
                    format!("FFMPEG version {:?}", ffmpeg_version),
                );
            } else {
                log_error(
                    tools::log::LogServiceType::Register,
                    "We were not able to confirm FFMPEG installation".to_string(),
                );
            }
            Ok::<(), RsError>(())
        });
    }

    let config = server::initialize_config().await;

    tokio::spawn(async {
        if let Err(error) = YydlContext::initialize().await {
            log_error(
                tools::log::LogServiceType::Register,
                format!("We were not able to prepare YT-DLP: {}", error),
            );
        }
    });

    if !config.imagesUseIm {
        log_info(
            tools::log::LogServiceType::Register,
            "Will use native libraries for image conversions".to_string(),
        );
    } else {
        log_info(
            tools::log::LogServiceType::Register,
            "Will use ImageMagick for image conversions".to_string(),
        );
    }

    let register_infos = register().await?;
    let (app, mc) = app().await?;
    let config = server::get_config().await;
    let signaling = webrtc::start_from_config(&config, app.clone());
    let local_port = get_server_port().await;

    // Certificates are chosen by SNI: the direct `*.<label>.servers.redseat.cloud` one
    // (hot-reloaded when renewed) and the legacy `<id>-srv.redseat.cloud` one.
    let resolver = Arc::new(direct::tls::SniResolver::default());
    if let Some((chain_path, key_path)) = &register_infos.cert_paths {
        match load_legacy_certificate(chain_path, key_path).await {
            Ok(certificate) => resolver.set_legacy(Some(certificate)),
            Err(error) => log_error(
                LogServiceType::Register,
                format!("Unable to load the legacy certificate: {:?}", error),
            ),
        }
    }
    let direct_enabled = direct::is_enabled(&config);
    // IPv6 gets its own listener (IPv6-only, so it never conflicts with the IPv4 one) for
    // the global IPv6 addresses reported to direct HTTPS clients.
    let ipv6_listener = if direct_enabled || resolver.has_certificate() {
        match bind_ipv6_only(local_port) {
            Ok(listener) => Some(listener),
            Err(error) => {
                log_info(
                    LogServiceType::Register,
                    format!("Not listening on IPv6: {}", error),
                );
                None
            }
        }
    } else {
        None
    };
    let direct_https = direct::start(&config, resolver.clone(), mc, ipv6_listener.is_some()).await;

    if direct_https || resolver.has_certificate() {
        log_info(
            tools::log::LogServiceType::Register,
            format!(
                "Starting HTTP/HTTPS server, TLS serves: {}",
                resolver.describe()
            ),
        );

        let tls_config = RustlsConfig::from_config(Arc::new(direct::tls::server_config(resolver)));

        //let addr = format!("[::]:{}", local_port).parse::<SocketAddr>().unwrap();
        let addr = SocketAddr::from(([0, 0, 0, 0], local_port));
        log_info(
            tools::log::LogServiceType::Register,
            format!("->> LISTENING HTTP/HTTPS on {:?}\n", addr),
        );

        let server = axum_server_dual_protocol::bind_dual_protocol(addr, tls_config.clone())
            .serve(app.clone().into_make_service());
        let server_ipv6 = async {
            match ipv6_listener {
                Some(listener) => {
                    log_info(
                        tools::log::LogServiceType::Register,
                        format!("->> LISTENING HTTP/HTTPS on [::]:{}\n", local_port),
                    );
                    axum_server_dual_protocol::from_tcp_dual_protocol(listener, tls_config)
                        .serve(app.into_make_service())
                        .await
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            result = server => result.unwrap(),
            result = server_ipv6 => result.unwrap(),
            _ = tokio::signal::ctrl_c() => {}
        }
    } else {
        log_info(
            tools::log::LogServiceType::Register,
            format!("Starting HTTP server only has no certificate found"),
        );
        let addr = SocketAddr::from(([0, 0, 0, 0], local_port));
        let listener = TcpListener::bind(addr).await.unwrap();
        log_info(
            LogServiceType::Register,
            format!("->> LISTENING on {:?}\n", listener.local_addr()),
        );

        let server = axum::serve(listener, app);
        tokio::select! {
            result = server => result.unwrap(),
            _ = tokio::signal::ctrl_c() => {}
        }
    }

    if let Some(signaling) = signaling {
        signaling.shutdown().await;
    }

    // endregion: --- Start Server

    Ok(())
}

fn bind_ipv6_only(port: u16) -> std::io::Result<std::net::TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_only_v6(true)?;
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
    socket.listen(1024)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

async fn load_legacy_certificate(
    chain_path: &PathBuf,
    key_path: &PathBuf,
) -> Result<direct::tls::SniCertificate> {
    let chain = tokio::fs::read_to_string(chain_path).await?;
    let key = tokio::fs::read_to_string(key_path).await?;
    direct::tls::SniCertificate::from_pem(&chain, &key)
}

async fn app() -> Result<(Router, ModelController)> {
    let store = SqliteStore::new().await.unwrap();
    let plugin_manager = PluginManager::new().await?;
    let mut mc = ModelController::new(store, plugin_manager).await?;

    let cors: CorsLayer = CorsLayer::new()
        .max_age(Duration::from_secs(3600))
        .allow_methods(vec![
            Method::GET,
            Method::PATCH,
            Method::DELETE,
            Method::HEAD,
            Method::OPTIONS,
            Method::POST,
            Method::PUT,
        ])
        // Echo requested headers: the web app sends Authorization, SHARETOKEN, Range…
        // (a `*` wildcard never covers Authorization).
        .allow_headers(AllowHeaders::mirror_request())
        .expose_headers([
            ACCEPT_RANGES,
            CONTENT_DISPOSITION,
            CONTENT_LENGTH,
            CONTENT_RANGE,
            CONTENT_TYPE,
        ])
        // Private Network Access: lets the public web app call this server on a LAN address.
        .allow_private_network(true)
        // Allow any origin: auth is token/session-based (not cookies),
        // and Chromecast HLS playback requires CORS from Google's receiver domain
        .allow_origin(Any);

    let server_id = get_server_id().await;
    let admin_users = mc
        .get_users(&model::users::ConnectedUser::ServerAdmin)
        .await?
        .into_iter()
        .filter(|u| u.is_admin())
        .collect::<Vec<_>>();
    if admin_users.is_empty() || server_id.is_none() {
        log_info(
            LogServiceType::Register,
            format!(
                "Register your server at: http://127.0.0.1:{}/infos/install",
                get_server_port().await
            ),
        );
    }
    let router = Router::new()
        .nest("/ping", routes::ping::routes())
        .nest("/infos", routes::infos::routes(mc.clone()))
        .nest("/libraries", routes::libraries::routes(mc.clone()))
        .nest(
            "/libraries/:libraryid/medias",
            routes::medias::routes(mc.clone()),
        )
        .nest(
            "/libraries/:libraryid/tags",
            routes::tags::routes(mc.clone()),
        )
        .nest(
            "/libraries/:libraryid/people",
            routes::people::routes(mc.clone()),
        )
        .nest(
            "/libraries/:libraryid/series",
            routes::series::routes(mc.clone()),
        )
        .nest(
            "/libraries/:libraryid/movies",
            routes::movies::routes(mc.clone()),
        )
        .nest(
            "/libraries/:libraryid/books",
            routes::books::routes(mc.clone()),
        )
        .nest(
            "/libraries/:libraryid/channels",
            routes::channels::routes(mc.clone()),
        )
        .nest(
            "/libraries/:libraryid/plugins",
            routes::library_plugins::routes(mc.clone()),
        )
        .nest("/libraries/:libraryid", routes::search::routes(mc.clone()))
        .nest("/library", routes::libraries::routes(mc.clone())) // duplicate for legacy
        .nest("/users", routes::users::routes(mc.clone()))
        .nest("/credentials", routes::credentials::routes(mc.clone()))
        .nest("/uploadkeys", routes::upload_keys::routes(mc.clone()))
        .nest("/backups", routes::backups::routes(mc.clone()))
        .nest("/plugins", routes::plugins::routes(mc.clone()))
        .nest("/sse", routes::sse::routes(mc.clone()))
        .route("/socket.io/", axum::routing::any(socket_io_fallback))
        .fallback(fallback)
        .layer(middleware::from_fn(mw_range::mw_range))
        //.layer(middleware::map_response(main_response_mapper))
        .layer(middleware::from_fn_with_state(
            mc.clone(),
            mw_auth::mw_token_resolver,
        ))
        .layer(DefaultBodyLimit::disable())
        .layer(ServiceBuilder::new().layer(cors))
        .layer(TraceLayer::new_for_http());
    Ok((router, mc))
}
async fn fallback(uri: Uri) -> (StatusCode, &'static str) {
    log_info(LogServiceType::Other, format!("Route not found: {}", uri));
    (StatusCode::NOT_FOUND, "Not Found")
}

/// Silent 404 for deprecated socket.io endpoint (old clients still call it)
async fn socket_io_fallback() -> StatusCode {
    StatusCode::NOT_FOUND
}
struct RegisterInfo {
    cert_paths: Option<(PathBuf, PathBuf)>,
}

async fn register() -> Result<RegisterInfo> {
    log_info(
        tools::log::LogServiceType::Register,
        "Checking registration".to_string(),
    );
    let config = get_config().await;
    if let Some(id) = config.id.clone() {
        log_info(
            tools::log::LogServiceType::Register,
            format!("Server ID: {}", id),
        );
    }
    let _ = get_or_init_keys().await;

    let mut register_info = RegisterInfo { cert_paths: None };

    if let (Some(id), Some(_)) = (config.id, config.token) {
        server::spawn_domain_reporter();
        if (config.noCert) {
            log_info(
                tools::log::LogServiceType::Register,
                "No Certificate option activated we will only expose http".to_string(),
            );
        } else if let Some(domain) = &config.domain {
            log_info(
                tools::log::LogServiceType::Register,
                format!(
                    "Custom domain {}: no RedSeat certificate (legacy or direct), TLS must be handled in front of the server",
                    domain
                ),
            );
        } else {
            log_info(
                tools::log::LogServiceType::Register,
                format!("Legacy certificate check ({}-srv.redseat.cloud)", id),
            );
            // Legacy `<id>-srv.redseat.cloud` certificate. Direct HTTPS doesn't depend on it,
            // so a failure here must not stop the server.
            match certificate::dns_certify().await {
                Ok(certs) => {
                    register_info.cert_paths = Some(certs.clone());
                    let public_config = PublicServerInfos::get(&certs.0, &id).await?;
                    log_info(
                        LogServiceType::Register,
                        format!(
                            "Legacy certificate ready: https://{}-srv.redseat.cloud:{}",
                            id, public_config.port
                        ),
                    );
                }
                Err(error) => log_error(
                    LogServiceType::Register,
                    format!("Legacy certificate unavailable: {:?}", error),
                ),
            }
        }
    }

    Ok(register_info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{self, header, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    // for `collect`
    use serde_json::{json, Value};
    use tower::ServiceExt; // for `call`, `oneshot`, and `ready`

    #[tokio::test]
    async fn routes() {
        let (router, _) = app().await.unwrap();

        // Test ping returns success JSON with CORS
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(http::Method::GET)
                    .uri("/ping")
                    .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .header(http::header::ORIGIN, "http://localhost:3000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "http://localhost:3000",
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body, json!({ "result": {"success": true} }));

        // Test unknown route returns 404
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn preflight_allows_private_network_and_range() {
        let (router, _) = app().await.unwrap();

        let response = router
            .oneshot(
                Request::builder()
                    .method(http::Method::OPTIONS)
                    .uri("/libraries/lib/medias/media")
                    .header(header::ORIGIN, "https://www.redseat.cloud")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .header(
                        header::ACCESS_CONTROL_REQUEST_HEADERS,
                        "authorization,range,sharetoken",
                    )
                    .header("Access-Control-Request-Private-Network", "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status().is_success());
        let headers = response.headers();
        assert_eq!(
            headers.get("Access-Control-Allow-Private-Network").unwrap(),
            "true"
        );
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_HEADERS).unwrap(),
            "authorization,range,sharetoken"
        );
    }
}
