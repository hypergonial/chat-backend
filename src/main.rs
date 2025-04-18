#![allow(async_fn_in_trait)]

use axum::{ServiceExt, extract::Request};
use chat_backend::{
    app::{App, ApplicationState},
    main_router,
};
use color_eyre::eyre::Result;
use mimalloc::MiMalloc;
use tokio::signal::ctrl_c;
use tower::Layer;
use tower_http::normalize_path::NormalizePathLayer;

#[cfg(debug_assertions)]
use tracing::level_filters::LevelFilter;

#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

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

    // gcp_auth requires a TLS provider to be installed
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install default TLS crypto provider.");

    // Initialize the application state
    let app = ApplicationState::from_env().await?;
    app.spawn_background_tasks();

    let router = main_router(app.clone());

    let listener = tokio::net::TcpListener::bind(app.config.listen_addr())
        .await
        .expect("Failed to bind address");

    tracing::info!("Listening on {}", app.config.listen_addr());

    axum::serve(
        listener,
        // voodoo magic to make trailing slashes go away from URLs
        ServiceExt::<Request>::into_make_service(NormalizePathLayer::trim_trailing_slash().layer(router)),
    )
    .with_graceful_shutdown(handle_signals(app))
    .await
    .expect("Failed creating server");

    Ok(())
}
