//! In-process gateway server launcher for e2e tests.
//!
//! Starts the **production router** (built by
//! [`gateway_example::gateway::build_gateway_with_settings`]) on a random
//! local port, with the DB pool wired through to the gateway state so the
//! DB-backed refresh-token rotation path is exercised. CSRF and session
//! middleware are therefore live — see [`Self::start`] for the bootstrap
//! dance required to obtain a CSRF token before POSTing.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::JoinHandle;

use tokio_util::sync::CancellationToken;

use gateway_example::gateway;
use gateway_example::module::ServiceModule;

/// Synthetic admin password used for in-test gateway launches.
///
/// The gateway `Settings` requires some non-empty `default_admin_password`; the
/// tests log in with this constant value via the gateway's HTTP login endpoint
/// (`POST /auth/login`).
pub const TEST_ADMIN_PASSWORD: &str = "synthetic-gateway-test-password";

/// RAII guard that runs a gateway server in the background on a random port.
///
/// # Errors
/// Returns an error if the server fails to start.
pub struct GatewayEnv {
    addr: SocketAddr,
    shutdown: CancellationToken,
    /// Held for its `Drop` side-effect: stops the Postgres testcontainer when
    /// the gateway env is dropped. Declared last so the container is dropped
    /// after the pool (struct fields drop in declaration order).
    #[allow(dead_code, reason = "Kept alive for Drop side-effect on GatewayEnv")]
    db: super::db::TestEnv,
}

impl GatewayEnv {
    /// Start a gateway server bound to a random local port.
    ///
    /// The admin password is injected directly via `Settings::load_dev_keys`
    /// rather than through `ADMIN_PASSWORD` env mutation — this keeps the test
    /// runner safe-by-default (no `unsafe`, no process-global state) and
    /// matches what production `--dev-keys` flow does after reading the env var
    /// itself.
    ///
    /// # Errors
    /// Returns an error if the Postgres testcontainer cannot start or its
    /// migrations fail, if dev-key generation or PEM encoding fails, or if the
    /// server fails to bind or become ready within 15 seconds.
    pub async fn start() -> Result<Self> {
        let settings = gateway_example::settings::Settings::load_dev_keys(TEST_ADMIN_PASSWORD)
            .context("failed to load dev keys for gateway")?;

        // Spin up a fresh Postgres testcontainer and run the gateway migrations
        // so the DB-backed refresh-token rotation path is live. The container
        // is kept alive by `Self.db` and stopped when the env is dropped.
        let db = super::db::TestEnv::postgres()
            .await
            .context("failed to start Postgres testcontainer for gateway")?;
        // The gateway migration is applied as inline DDL instead of via
        // `sqlx::migrate!`: the testcontainer DB is shared with
        // `TestEnv::postgres()`, which already ran the live-search migrations
        // (versions 1-3) into the `_sqlx_migrations` table. A second migrator
        // whose resolved set ({100}) omits those versions would fail with
        // "migration 1 was previously applied but is missing in the resolved
        // migrations". Running the single gateway migration as raw SQL bypasses
        // the `_sqlx_migrations` bookkeeping entirely.
        sqlx::raw_sql(include_str!(
            "../../../gateway/migrations/100_create_refresh_tokens.up.sql"
        ))
        .execute(db.pool())
        .await
        .context("failed to apply gateway refresh_tokens schema")?;

        let modules: Vec<Arc<dyn ServiceModule>> = vec![
            Arc::new(gateway_example::services::search::SearchService),
            Arc::new(gateway_example::services::proxy::ProxyService),
            Arc::new(gateway_example::services::monitor::MonitorService),
        ];

        // The production router builder owns session/CSRF/governor wiring.
        // The DB pool is wired through so `/auth/login` can issue refresh
        // tokens against the real `refresh_tokens` table.
        let app = gateway::build_gateway_with_settings(
            modules,
            settings,
            "https://ipapi.co".to_string(),
            Some(db.pool().clone()),
            60 * 60 * 24 * 30, // refresh TTL
            15 * 60,             // access JWT TTL
        )
        .context("failed to build gateway router")?;

        let shutdown = CancellationToken::new();
        let serve_token = shutdown.clone();

        // Bind and serve on a dedicated runtime thread so the server
        // survives individual test runtimes being dropped.
        let bound_addr: SocketAddr = {
            let addr_lock = Arc::new(std::sync::Mutex::new(None::<SocketAddr>));
            let addr_clone = Arc::clone(&addr_lock);

            let thread: JoinHandle<Result<()>> = std::thread::Builder::new()
                .name("gateway-server".into())
                .spawn(move || -> Result<()> {
                    let rt = tokio::runtime::Runtime::new()
                        .context("failed to create gateway server runtime")?;
                    rt.block_on(async move {
                        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                            .await
                            .context("Failed to bind gateway listener")?;
                        let bound = listener
                            .local_addr()
                            .context("Failed to get bound address")?;
                        addr_clone
                            .lock()
                            .map_err(|e| anyhow::anyhow!("addr lock poisoned: {e}"))?
                            .replace(bound);
                        // Propagate axum errors instead of swallowing.
                        axum::serve(
                            listener,
                            app.into_make_service_with_connect_info::<SocketAddr>(),
                        )
                        .with_graceful_shutdown(async move {
                            serve_token.cancelled().await;
                        })
                        .await
                        .context("gateway axum server exited with error")?;
                        Ok::<_, anyhow::Error>(())
                    })
                })
                .context("Failed to spawn gateway server thread")?;

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let addr = loop {
                {
                    let guard = addr_lock
                        .lock()
                        .map_err(|e| anyhow::anyhow!("addr lock poisoned: {e}"))?;
                    if let Some(addr) = *guard {
                        break addr;
                    }
                }
                if std::time::Instant::now() >= deadline {
                    return Err(anyhow::anyhow!("Gateway server did not bind within 15s"));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            };
            // Stash the JoinHandle so Drop can detach cleanly.
            // (Drop is sync; we can't await it from here. The thread exits
            // when the test process exits.)
            let _ = thread;
            addr
        };

        // Wait for the server to be ready (health check).
        let health_url = format!("http://{bound_addr}/health");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .context("Failed to build health check client")?;
        let health_deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Ok(resp) = client.get(&health_url).send().await
                && resp.status().is_success()
            {
                break;
            }
            if std::time::Instant::now() >= health_deadline {
                return Err(anyhow::anyhow!(
                    "Gateway server at {bound_addr} did not become ready within 15s"
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Ok(Self {
            addr: bound_addr,
            shutdown,
            db,
        })
    }

    /// Returns the base URL of the running gateway (e.g. `http://127.0.0.1:54321`).
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Returns the bound [`SocketAddr`].
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Cancel the graceful-shutdown token.
    pub async fn shutdown(self) {
        self.shutdown.cancel();
        // Allow a brief moment for the server to drain.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

impl Drop for GatewayEnv {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

impl std::fmt::Debug for GatewayEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayEnv")
            .field("addr", &self.addr)
            .field("shutdown", &"<cancellation token>")
            .field("db", &self.db)
            .finish()
    }
}