use std::time::Duration;

use axum::{Json, Router, extract::State, routing::get};
use http::{Method, header};
use serde_json::{Value, json};
use tower_http::cors::{Any, CorsLayer};

use crate::app::App;

use super::channels::get_router as get_channel_router;
use super::guilds::get_router as get_guild_router;
use super::prefs::get_router as get_prefs_router;
use super::users::get_router as get_user_router;

/// Get all routes for the REST API. Includes CORS.
pub fn get_router() -> Router<App> {
    // https://javascript.info/fetch-crossorigin
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS
    let cors = CorsLayer::new()
        // TODO: Change this to the actual origin
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::DELETE,
            Method::OPTIONS,
            Method::PUT,
            Method::PATCH,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::ORIGIN,
            header::AUTHORIZATION,
            header::CACHE_CONTROL,
        ])
        .max_age(Duration::from_secs(3600));

    get_channel_router()
        .merge(get_guild_router())
        .merge(get_user_router())
        .merge(get_prefs_router())
        .route("/", get(get_api_root))
        .layer(cors)
}

async fn get_api_root(State(app): State<App>) -> Json<Value> {
    Json(json!({
        "capabilities": app.ops().get_capabilities(),
    }))
}
