//! In-process gateway server launcher for e2e tests.
//!
//! Starts the production gateway router on a random local port with a fresh
//! Postgres testcontainer. `TestEnv::postgres` applies the same shared workspace
//! migration history used by both production services.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::JoinHandle;

use tokio_util::sync::CancellationToken;

use gateway_example::gateway;
use gateway_example::module::ServiceModule;

pub const TEST_ADMIN_PASSWORD: &str = "synthetic-gateway-test-password";
pub const TEST_ADMIN_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

pub struct GatewayEnv {
    addr: SocketAddr,
    shutdown: CancellationToken,
    #[allow(dead_code, reason = "Kept alive for Drop side-effect on GatewayEnv")]
    db: super::db::TestEnv,
}

impl GatewayEnv {
    /// Start a gateway server bound to a random local port.
    ///
    /// # Errors
    /// Returns an error if Postgres, migrations, key generation, binding, or
    /// readiness checks fail.
    pub async fn start() -> Result<Self> {
        let settings = gateway_example::settings::Settings::load_dev_keys(TEST_ADMIN_PASSWORD)
            .context("failed to load dev keys for gateway")?;

        // This now applies search + gateway migrations in one SQLx history; no
        // raw-DDL bypass of `_sqlx_migrations` is necessary or desirable.
        let db = super::db::TestEnv::postgres()
            .await
            .context("failed to start Postgres testcontainer for gateway")?;

        let modules: Vec<Arc<dyn ServiceModule>> = vec![
            Arc::new(gateway_example::services::search::SearchService),
            Arc::new(gateway_example::services::proxy::ProxyService),
            Arc::new(gateway_example::services::monitor::MonitorService),
        ];

        let app = gateway::build_gateway_with_settings(
            modules,
            settings,
            "https://ipapi.co".to_string(),
            Some(db.pool().clone()),
            60 * 60 * 24 * 30,
            15 * 60,
        )
        .context("failed to build gateway router")?;

        let shutdown = CancellationToken::new();
        let serve_token = shutdown.clone();

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
            drop(thread);
            addr
        };

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

    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn shutdown(self) {
        self.shutdown.cancel();
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
