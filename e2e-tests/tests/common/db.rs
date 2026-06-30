//! Database test helpers — per-test Postgres isolation via testcontainers.
//!
//! Always spins up a fresh Postgres 17 container. The `DATABASE_URL` env var is
//! intentionally ignored — tests are strictly container-based for isolation.
//! Container lifecycle is bound to [`TestEnv`] RAII (dropped on test exit).

// Each e2e-tests/tests/*.rs binary compiles its own copy; not every helper is
// used by every binary, so suppressing dead_code at the module level is cleaner
// than annotating every struct/function individually.
#![allow(
    dead_code,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "Some helpers unused per test-binary compilation; \
              test-support code uses expect/unwrap/panic for fail-fast assertions"
)]

use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// RAII guard for a test-scoped Postgres database.
///
/// The container is dropped (and thus stopped) when the test completes, even on
/// panic via `Drop`.
pub struct TestEnv {
    pool: PgPool,
    connection_string: String,
    #[allow(
        dead_code,
        reason = "Field stays alive for RAII; never read after construction"
    )]
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
    /// Start a fresh Postgres 17 container, connect, run all migrations.
    ///
    /// The `DATABASE_URL` environment variable is **not** consulted — this
    /// function always uses testcontainers for reliable isolation.
    ///
    /// # Panics
    /// Panics if the container cannot start, the pool cannot connect, or
    /// migrations fail.
    pub async fn postgres() -> Self {
        let container = Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await
            .expect("Failed to start Postgres testcontainer");

        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get Postgres host port");

        let connection_string =
            format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");

        let pool = PgPool::connect(&connection_string)
            .await
            .expect("Failed to connect to Postgres testcontainer");

        // Run live-search migrations to create the search_results table.
        // NOTE: gateway migrations (refresh_tokens table) are NOT run here
        // because they use overlapping version numbers in the shared
        // _sqlx_migrations table. Gateway-specific e2e tests target the real
        // gateway service via HTTP, not this pool.
        sqlx::migrate!("../live-search/migrations")
            .run(&pool)
            .await
            .expect("Failed to run live-search migrations");

        Self {
            pool,
            connection_string,
            container: Box::new(container),
        }
    }

    /// Borrow the database pool.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The connection string used to connect to the testcontainer.
    #[must_use]
    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }
}
