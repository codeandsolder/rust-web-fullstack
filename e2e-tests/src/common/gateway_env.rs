//! In-process gateway server launcher for e2e tests.
//!
//! Starts the production gateway router on a random local port with a fresh
//! Postgres testcontainer. `TestEnv::postgres` applies the same shared workspace
//! migration history used by both production services.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use tokio_util::sync::CancellationToken;

use gateway_example::gateway;
use gateway_example::module::ServiceModule;

pub const TEST_ADMIN_PASSWORD: &str = "synthetic-gateway-test-password";
pub const TEST_ADMIN_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

pub struct GatewayEnv {
    addr: SocketAddr,
    shutdown: CancellationToken,
    server: JoinHandle<Result<()>>,
    db: super::db::TestEnv,
}

impl GatewayEnv {
    /// Start a gateway server bound to a random local port.
    ///
    /// # Errors
    /// Returns an error if Postgres, migrations, key generation, binding, or
    /// readiness checks fail.
    pub async fn start() -> Result<Self> {
        let config = rwf_config::GatewayConfig {
            admin_user_id: TEST_ADMIN_USER_ID.to_owned(),
            session: rwf_config::SessionConfig {
                cookie_secure: false,
            },
            ..rwf_config::GatewayConfig::default()
        };
        let settings =
            gateway_example::settings::Settings::generate_dev_keys(TEST_ADMIN_PASSWORD, &config)
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
            config.proxy_upstream_url,
            Some(db.pool().clone()),
        )
        .context("failed to build gateway router")?;

        // SharedServer already owns a long-lived Tokio runtime. Bind before
        // spawning, so startup neither needs another runtime nor polls a mutex.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind gateway listener")?;
        let addr = listener
            .local_addr()
            .context("failed to get gateway address")?;
        let shutdown = CancellationToken::new();
        let serve_token = shutdown.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(serve_token.cancelled_owned())
            .await
            .context("gateway axum server exited with error")
        });

        let env = Self {
            addr,
            shutdown,
            server,
            db,
        };
        env.wait_ready().await?;
        Ok(env)
    }

    async fn wait_ready(&self) -> Result<()> {
        let health_url = format!("{}/health", self.base_url());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .context("failed to build health check client")?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(resp) = client.get(&health_url).send().await
                && resp.status().is_success()
            {
                return Ok(());
            }
            anyhow::ensure!(
                !self.server.is_finished() && tokio::time::Instant::now() < deadline,
                "gateway at {} did not become ready within 15s",
                self.addr
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[must_use = "the returned URL identifies the test gateway endpoint"]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    #[must_use = "the returned address identifies the bound gateway socket"]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop accepting requests and wait for the actual server task to exit.
    ///
    /// # Errors
    /// Returns an error if shutdown times out or the server task fails.
    pub async fn shutdown(mut self) -> Result<()> {
        self.shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), &mut self.server)
            .await
            .context("gateway graceful shutdown timed out")?
            .context("gateway server task failed")?
    }
}

impl Drop for GatewayEnv {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.server.abort();
    }
}

impl std::fmt::Debug for GatewayEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayEnv")
            .field("addr", &self.addr)
            .field("shutdown", &"<cancellation token>")
            .field("server", &self.server)
            .field("db", &self.db)
            .finish()
    }
}
