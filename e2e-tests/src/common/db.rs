//! Database test helpers — per-test Postgres isolation via testcontainers.
//!
//! Always spins up a fresh Postgres 17 container. The `DATABASE_URL` env var is
//! intentionally ignored — tests are strictly container-based for isolation.
//! Container lifecycle is bound to [`TestEnv`] RAII (dropped on test exit).

use anyhow::{Context, Result};
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// RAII guard for a test-scoped Postgres database.
pub struct TestEnv {
    pool: PgPool,
    connection_string: String,
    #[allow(dead_code, reason = "Kept alive for Drop side-effect on TestEnv")]
    container: Box<ContainerAsync<Postgres>>,
}

impl std::fmt::Debug for TestEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestEnv")
            .field("pool", &self.pool)
            .field("connection_string", &self.connection_string)
            .field("container", &"<container>")
            .finish()
    }
}

impl TestEnv {
    /// Start a fresh Postgres 17 container, connect, run the workspace's
    /// complete migration history, and insert a small deterministic fixture row.
    ///
    /// The fixture makes browser tests independent of libtest execution order;
    /// tests that need additional rows should still insert their own data.
    ///
    /// # Errors
    /// Returns an error if the container cannot start, the pool cannot connect,
    /// migrations fail, or the fixture row cannot be inserted.
    pub async fn postgres() -> Result<Self> {
        let container = Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await
            .context("Failed to start Postgres testcontainer")?;

        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .context("Failed to get Postgres host port")?;

        let connection_string =
            format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");

        let pool = PgPool::connect(&connection_string)
            .await
            .context("Failed to connect to Postgres testcontainer")?;

        sqlx::migrate!("../migrations")
            .run(&pool)
            .await
            .context("Failed to run workspace migrations")?;

        // Browser search tests must not depend on another integration test
        // having happened to seed the shared fixture first. Use a URL that is
        // distinct from the per-test rows in live_search_test.rs so the URL
        // uniqueness constraint remains useful.
        sqlx::query(
            "INSERT INTO search_results (title, url, snippet) \
             VALUES ($1, $2, $3) ON CONFLICT (url) DO NOTHING",
        )
        .bind("Rust Browser Fixture")
        .bind("https://fixture.invalid/rust-browser")
        .bind("Deterministic Rust search result for browser end-to-end tests")
        .execute(&pool)
        .await
        .context("Failed to seed deterministic browser fixture")?;

        Ok(Self {
            pool,
            connection_string,
            container: Box::new(container),
        })
    }

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    #[must_use]
    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }
}
