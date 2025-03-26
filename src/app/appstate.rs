use std::{net::SocketAddr, sync::Arc};

use aws_config::BehaviorVersion;
use aws_sdk_s3::{
    Client, Config as S3Config,
    config::{Credentials as S3Creds, Region},
};

use derive_builder::Builder;
use dotenvy::dotenv;
use secrecy::{ExposeSecret, Secret};

use super::ops::Ops;
use crate::{external::FirebaseMessaging, models::errors::BuildError};
use crate::{
    external::{Database, S3Service},
    gateway::handler::Gateway,
    models::errors::AppError,
};

pub type App = Arc<ApplicationState>;
pub type S3Client = Client;

/// Contains all the application state and manages application state changes.
pub struct ApplicationState {
    db: Database,
    gateway: Gateway,
    pub config: Config,
    s3: Option<S3Service>,
    fcm: Option<FirebaseMessaging>,
}

impl ApplicationState {
    /// Create a new application state from environment variables.
    ///
    /// ## Errors
    ///
    /// * [`AppError`] - If the application fails to initialize.
    ///
    /// ## Returns
    ///
    /// A new application state wrapped in an `Arc`.
    pub async fn from_env() -> Result<Arc<Self>, AppError> {
        let config = Config::from_env();

        let s3 = {
            config.s3_config().map_or_else(
                || {
                    tracing::warn!("S3 not configured - File uploads will be unavailable.");
                    None
                },
                |s3_config| {
                    let s3creds = S3Creds::new(
                        s3_config.access_key().expose_secret(),
                        s3_config.secret_key().expose_secret(),
                        None,
                        None,
                        "chat",
                    );

                    let s3conf = S3Config::builder()
                        .region(Region::new(s3_config.region().to_string()))
                        .endpoint_url(s3_config.url())
                        .credentials_provider(s3creds)
                        .force_path_style(true) // MinIO does not support virtual hosts
                        .behavior_version(BehaviorVersion::v2025_01_17())
                        .build();

                    Some(S3Service::new(Client::from_conf(s3conf)))
                },
            )
        };

        let fcm = match FirebaseMessaging::new() {
            Ok(fcm) => Some(fcm),
            Err(e) => {
                tracing::warn!("Failed to initialize Firebase Messaging - Push Notifications will be unavailable: {e}");
                None
            }
        };

        let mut state = Self {
            db: Database::new(),
            gateway: Gateway::new(),
            fcm,
            config,
            s3,
        };

        state.init().await?;

        Ok(Arc::new_cyclic(|w| {
            state.db.bind_to(w.clone());
            if let Some(s3) = &mut state.s3 {
                s3.bind_to(w.clone());
            }
            state.gateway.bind_to(w.clone());
            state.gateway.start();
            state
        }))
    }

    /// Create a new application state from the individual components.
    ///
    /// ## Errors
    ///
    /// * [`AppError`] - If the application fails to initialize.
    ///
    /// ## Returns
    ///
    /// A new application state wrapped in an `Arc`.
    pub async fn from_components(
        db: Database,
        gateway: Gateway,
        config: Config,
        s3: Option<S3Service>,
        fcm: Option<FirebaseMessaging>,
    ) -> Result<Arc<Self>, AppError> {
        let mut state = Self {
            db,
            gateway,
            config,
            s3,
            fcm,
        };

        state.init().await?;

        let shared_state = Arc::new_cyclic(|w| {
            state.db.bind_to(w.clone());
            if let Some(s3) = &mut state.s3 {
                s3.bind_to(w.clone());
            }
            state.gateway.bind_to(w.clone());
            state.gateway.start();
            state
        });

        Ok(shared_state)
    }

    /// Initializes the application
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database connection fails.
    async fn init(&mut self) -> Result<(), AppError> {
        self.db.connect(self.config.database_url().expose_secret()).await?;
        tracing::info!("Database is ready.");
        if let Some(s3) = self.s3.as_mut() {
            s3.create_buckets().await?;
            tracing::info!("S3 is ready.");
        }
        Ok(())
    }

    /// Spawn maintenance tasks to run in the background.
    pub fn spawn_background_tasks(self: &Arc<Self>) {
        let app = self.clone();

        tokio::spawn(async move {
            loop {
                tracing::info!("Clearing stale FCM tokens...");
                match app.ops().clear_stale_fcm_tokens().await {
                    Ok(count) => {
                        tracing::info!("Cleared {} stale FCM tokens.", count);
                    }
                    Err(e) => {
                        tracing::error!("Failed to clear stale FCM tokens: {}", e);
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(3600 * 24 /* 1 day */)).await;
            }
        });
    }

    /// The gateway instance of the application.
    #[inline]
    pub const fn gateway(&self) -> &Gateway {
        &self.gateway
    }

    /// The S3 client instance of the application.
    #[inline]
    pub const fn s3(&self) -> Option<&S3Service> {
        self.s3.as_ref()
    }

    /// The database instance of the application.
    #[inline]
    pub const fn db(&self) -> &Database {
        &self.db
    }

    /// Closes the application and cleans up resources.
    pub async fn close(&self) {
        self.gateway().stop().await;
        self.db().close().await;
    }

    #[inline]
    pub const fn ops(&self) -> Ops {
        Ops::new(
            &self.db,
            &self.config,
            self.s3.as_ref(),
            Some(&self.gateway),
            self.fcm.as_ref(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct S3EnvConfig {
    url: String,
    region: String,
    access_key: Secret<String>,
    secret_key: Secret<String>,
}
impl S3EnvConfig {
    pub const fn new(url: String, region: String, access_key: Secret<String>, secret_key: Secret<String>) -> Self {
        Self {
            url,
            region,
            access_key,
            secret_key,
        }
    }

    /// The URL of the S3 instance.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The region of the S3 instance to connect to.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// The access key for the S3 instance.
    pub const fn access_key(&self) -> &Secret<String> {
        &self.access_key
    }

    /// The secret key for the S3 instance.
    pub const fn secret_key(&self) -> &Secret<String> {
        &self.secret_key
    }

    /// Try to resolve the S3 configuration from environment variables.
    fn from_env() -> Option<Self> {
        let get_env_var = |name: &str| -> Option<String> {
            match std::env::var(name).ok() {
                None => {
                    tracing::warn!("{} environment variable not set", name);
                    None
                }
                Some(v) if v.is_empty() => {
                    tracing::warn!("{} environment variable is not set", name);
                    None
                }
                Some(v) => Some(v),
            }
        };

        let url = get_env_var("S3_URL")?;
        let region = get_env_var("S3_REGION")?;
        let access_key = get_env_var("S3_ACCESS_KEY").map(Secret::new)?;
        let secret_key = get_env_var("S3_SECRET_KEY").map(Secret::new)?;

        Some(Self {
            url,
            region,
            access_key,
            secret_key,
        })
    }
}

/// Application configuration
#[derive(Debug, Clone, Builder)]
#[builder(setter(into), build_fn(error = "BuildError"))]
pub struct Config {
    database_url: Secret<String>,
    s3_config: Option<S3EnvConfig>,
    listen_addr: SocketAddr,
    machine_id: i32,
    process_id: i32,
    app_secret: Secret<String>,
}

impl Config {
    /// Create a new builder to construct a [`Config`].
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// The database URL.
    pub const fn database_url(&self) -> &Secret<String> {
        &self.database_url
    }

    /// The URL for the `MinIO` server, an S3-compatible storage backend.
    pub const fn s3_config(&self) -> Option<&S3EnvConfig> {
        self.s3_config.as_ref()
    }

    /// The machine id.
    pub const fn machine_id(&self) -> i32 {
        self.machine_id
    }

    /// The process id.
    pub const fn process_id(&self) -> i32 {
        self.process_id
    }

    /// The addres for the backend server to listen on.
    pub const fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    /// APP secret used to create JWT tokens.
    pub const fn app_secret(&self) -> &Secret<String> {
        &self.app_secret
    }

    /// Creates a new config from environment variables
    ///
    /// ## Panics
    ///
    /// Panics if any of the required environment variables are not set
    /// or if they are not in a valid format.
    pub fn from_env() -> Self {
        dotenv().ok();
        Self::builder()
            .database_url(std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable must be set"))
            .s3_config(S3EnvConfig::from_env())
            .machine_id(
                std::env::var("MACHINE_ID")
                    .expect("MACHINE_ID environment variable must be set")
                    .parse::<i32>()
                    .expect("MACHINE_ID must be a valid integer"),
            )
            .process_id(
                std::env::var("PROCESS_ID")
                    .expect("PROCESS_ID environment variable must be set")
                    .parse::<i32>()
                    .expect("PROCESS_ID must be a valid integer"),
            )
            .listen_addr(
                std::env::var("LISTEN_ADDR")
                    .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
                    .parse::<SocketAddr>()
                    .expect("LISTEN_ADDR must be a valid socket address"),
            )
            .app_secret(std::env::var("APP_SECRET").expect("APP_SECRET environment variable must be set"))
            .build()
            .expect("Failed to create application configuration.")
    }
}
