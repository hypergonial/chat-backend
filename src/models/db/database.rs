use std::sync::{Arc, Weak};

use sqlx::{Executor, migrate, postgres::PgPool};

use crate::models::state::ApplicationState;

#[derive(Clone, Debug)]
pub struct Database {
    pool: Option<PgPool>,
    app: Weak<ApplicationState>,
}

impl Database {
    /// Creates a new database instance
    ///
    /// Note: The database is not connected by default
    pub const fn new() -> Self {
        Self {
            pool: None,
            app: Weak::new(),
        }
    }

    /// Creates a new database instance with an already connected pool
    ///
    /// ## Arguments
    ///
    /// * `pool` - The connected database pool
    pub const fn from_pool(pool: PgPool) -> Self {
        Self {
            pool: Some(pool),
            app: Weak::new(),
        }
    }

    pub fn bind_to(&mut self, app: Weak<ApplicationState>) {
        self.app = app;
    }

    pub fn app(&self) -> Arc<ApplicationState> {
        self.app.upgrade().expect("Application state has been dropped.")
    }

    /// The database pool
    ///
    /// ## Panics
    ///
    /// If the database is not connected
    pub const fn pool(&self) -> &PgPool {
        self.pool
            .as_ref()
            .expect("Database is not connected or has been closed.")
    }

    /// Checks if the database is connected
    ///
    /// ## Returns
    ///
    /// `true` if the database is connected, `false` otherwise
    pub fn is_connected(&self) -> bool {
        self.pool.as_ref().is_some_and(|pool| !pool.is_closed())
    }

    /// Connects to the database. Calls to a connected database are ignored.
    ///
    /// ## Arguments
    ///
    /// * `url` - The postgres connection URL
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database connection fails
    pub async fn connect(&mut self, url: &str) -> Result<(), sqlx::Error> {
        if let Some(pool) = &self.pool {
            if !pool.is_closed() {
                return Ok(());
            }
        }

        self.pool = Some(PgPool::connect(url).await?);
        migrate!("./migrations").run(self.pool()).await?;
        Ok(())
    }

    /// Closes the database connection
    pub async fn close(&self) {
        self.pool().close().await;
    }
}

// Allow the Database instance to be used as an executor directly
impl<'c> Executor<'c> for &Database {
    type Database = sqlx::Postgres;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> futures::stream::BoxStream<
        'e,
        Result<
            sqlx::Either<<Self::Database as sqlx::Database>::QueryResult, <Self::Database as sqlx::Database>::Row>,
            sqlx::Error,
        >,
    >
    where
        'c: 'e,
        E: 'q + sqlx::Execute<'q, Self::Database>,
    {
        self.pool().fetch_many(query)
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> futures::future::BoxFuture<'e, Result<Option<<Self::Database as sqlx::Database>::Row>, sqlx::Error>>
    where
        'c: 'e,
        E: 'q + sqlx::Execute<'q, Self::Database>,
    {
        self.pool().fetch_optional(query)
    }

    fn prepare_with<'e, 'q: 'e>(
        self,
        sql: &'q str,
        parameters: &'e [<Self::Database as sqlx::Database>::TypeInfo],
    ) -> futures::future::BoxFuture<'e, Result<<Self::Database as sqlx::Database>::Statement<'q>, sqlx::Error>>
    where
        'c: 'e,
    {
        self.pool().prepare_with(sql, parameters)
    }

    fn describe<'e, 'q: 'e>(
        self,
        sql: &'q str,
    ) -> futures::future::BoxFuture<'e, Result<sqlx::Describe<Self::Database>, sqlx::Error>>
    where
        'c: 'e,
    {
        self.pool().describe(sql)
    }
}

impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}
