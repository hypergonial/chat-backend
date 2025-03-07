#![allow(async_fn_in_trait)]

pub mod gateway;
pub mod models;
pub mod rest;
pub mod utils;

use axum::{Router, ServiceExt, extract::Request};
use color_eyre::eyre::Result;
use mimalloc::MiMalloc;
use models::state::App;
use tokio::signal::ctrl_c;
use tower::Layer;
use tower_http::{normalize_path::NormalizePathLayer, trace::TraceLayer};

#[cfg(debug_assertions)]
use tracing::level_filters::LevelFilter;

#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

use crate::models::state::ApplicationState;

// See: https://github.com/microsoft/mimalloc
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[cfg(unix)]
async fn handle_signals(state: App) {
    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to create SIGTERM signal listener");

    tokio::select! {
        _ = sigterm.recv() => {
            tracing::info!("Received SIGTERM, terminating...");
        }
        _ = ctrl_c() => {
            tracing::info!("Received keyboard interrupt, terminating...");
        }
    };
    state.close().await;
}

#[cfg(not(unix))]
async fn handle_signals(state: App) {
    ctrl_c().await.expect("Failed to create CTRL+C signal listener");
    state.close().await;
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    #[cfg(debug_assertions)]
    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_target(false)
        .with_max_level(LevelFilter::DEBUG)
        .without_time()
        .finish();

    #[cfg(not(debug_assertions))]
    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_target(false)
        .without_time()
        .finish();

    /* console_subscriber::init(); */
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set subscriber");

    // Initialize the application state
    let state = ApplicationState::from_env().await?;

    let router = Router::new()
        .nest("/gateway/v1", gateway::handler::get_router())
        .nest("/api/v1", rest::routes::get_router())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(state.config.listen_addr())
        .await
        .expect("Failed to bind address");

    tracing::info!("Listening on {}", state.config.listen_addr());

    axum::serve(
        listener,
        // voodoo magic to make trailing slashes go away from URLs
        ServiceExt::<Request>::into_make_service(NormalizePathLayer::trim_trailing_slash().layer(router)),
    )
    .with_graceful_shutdown(handle_signals(state))
    .await
    .expect("Failed creating server");

    Ok(())
}
