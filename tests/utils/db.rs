use std::net::SocketAddr;

use chat_backend::{
    app::{Config, ops::Ops},
    external::Database,
};
use secrecy::Secret;
use sqlx::PgPool;

/// A mock application that can use `.ops()` to access database operations.
pub struct DBApp {
    db: Database,
    config: Config,
}

impl DBApp {
    /// Create a new [`DBApp`] with the given database pool.
    ///
    /// ## Arguments
    ///
    /// * `pool` - The database pool to use for the application.
    ///
    /// ## Returns
    ///
    /// The newly created [`DBApp`].
    pub fn new(pool: PgPool) -> Self {
        Self {
            db: Database::from_pool(pool),
            config: Config::builder()
                .database_url(Secret::new(String::new()))
                .s3_url(String::new())
                .s3_access_key(Secret::new(String::new()))
                .s3_secret_key(Secret::new(String::new()))
                .s3_region(String::new())
                .listen_addr("127.0.0.1:8080".parse::<SocketAddr>().expect("Not valid SocketAddr"))
                .machine_id(0)
                .process_id(0)
                .app_secret(Secret::new(String::new()))
                .build()
                .expect("Failed to build Config"),
        }
    }

    /// The Ops struct for this application.
    pub const fn ops(&self) -> Ops<'_> {
        Ops::new(&self.db, &self.config, None, None, None)
    }

    pub const fn config(&self) -> &Config {
        &self.config
    }
}
