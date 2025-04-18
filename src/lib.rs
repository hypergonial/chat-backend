use app::App;
use axum::Router;
use tower_http::trace::TraceLayer;

pub mod app;
pub mod external;
pub mod gateway;
pub mod models;
pub mod rest;
pub mod utils;

/// The main router for the application.
/// This is the entry point for all requests.
/// It handles the routing of requests to the appropriate handlers.
///
/// # Arguments
///
/// * `state` - The application state to be passed to the handlers.
pub fn main_router(state: App) -> Router {
    Router::new()
        .nest("/gateway/v1", gateway::handler::get_router())
        .nest("/api/v1", rest::routes::get_router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
