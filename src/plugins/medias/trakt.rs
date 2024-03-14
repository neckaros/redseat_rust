use trakt_rs::{Request, Response};
use crate::Result;
// Context required for all requests
static ctx: trakt_rs::Context = trakt_rs::Context {
    base_url: "https://api.trakt.tv",
    client_id: "client_id",
    oauth_token: None,
};


async fn get_movie() -> Result<()> {
    // Create a request and convert it into an HTTP request
    let req = trakt_rs::api::movies::summary::Request {
        id: "tt123456".to_string(),
    };
    let http_req: http::Request<Vec<u8>> = req.try_into_http_request(ctx).unwrap();

    Ok(())

}