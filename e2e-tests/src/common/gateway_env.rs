//! In-process gateway server launcher for e2e tests.
//!
//! Starts a minimal gateway on a random local port.  Does NOT include CSRF /
//! session / governor middleware so that auth tests can POST without CSRF
//! tokens.  The server is cleaned up when [`GatewayEnv`] is dropped.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router, middleware,
    routing::{get, post},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

use gateway_example::gateway::{GatewayState, health_handler, root_handler};
use gateway_example::module::{ServiceInfo, ServiceModule};
use gateway_example::sse::GatewayEvent;

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
    /// Returns an error if dev-key generation or PEM encoding fails, or if the
    /// server fails to bind or become ready within 15 seconds.
    pub async fn start() -> Result<Self> {
        let settings = gateway_example::settings::Settings::load_dev_keys(TEST_ADMIN_PASSWORD)
            .context("failed to load dev keys for gateway")?;

        let modules: Vec<Arc<dyn ServiceModule>> = vec![
            Arc::new(gateway_example::services::search::SearchService),
            Arc::new(gateway_example::services::proxy::ProxyService),
            Arc::new(gateway_example::services::monitor::MonitorService),
        ];

        let app = build_test_gateway(modules, settings);

        let shutdown = CancellationToken::new();
        let serve_token = shutdown.clone();

        // Bind and serve on a dedicated runtime thread so the server
        // survives individual test runtimes being dropped.
        let bound_addr: SocketAddr = {
            let addr_lock = Arc::new(std::sync::Mutex::new(None::<SocketAddr>));
            let addr_clone = Arc::clone(&addr_lock);

            std::thread::Builder::new()
                .name("gateway-server".into())
                .spawn(move || -> Result<()> {
                    let rt = tokio::runtime::Runtime::new()
                        .context("failed to create gateway server runtime")?;
                    rt.block_on(async {
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
                        .ok();
                        Ok::<_, anyhow::Error>(())
                    })
                })
                .context("Failed to spawn gateway server thread")?;

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
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
            }
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

    /// Cancel the graceful-shutdown token and await server termination.
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
            .finish()
    }
}

// ──── Test gateway router ─────────────────────────────────────────────────

/// Build a minimal gateway Router without CSRF / session / governor middleware.
///
/// This lets tests run auth flows without needing to fetch and echo CSRF tokens.
fn build_test_gateway(
    modules: Vec<Arc<dyn ServiceModule>>,
    settings: gateway_example::settings::Settings,
) -> Router {
    let (tx, _rx) = broadcast::channel::<GatewayEvent>(100);

    let service_infos: Vec<ServiceInfo> = modules
        .iter()
        .map(|m| ServiceInfo {
            name: m.name(),
            path: m.path(),
            description: m.description(),
            enabled: m.enabled(),
        })
        .collect();

    // Nest each service's router.
    let mut service_router: Router<GatewayState> = Router::new();
    for module in &modules {
        if module.enabled() {
            service_router = service_router.nest(&format!("/{}", module.path()), module.router());
        }
    }

    let state = GatewayState {
        tx,
        services: service_infos,
        modules,
        settings,
    };

    let cors = gateway_example::cors::cors_layer();

    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/events", get(gateway_example::sse::sse_handler))
        .route("/auth/login", post(gateway_example::auth::login_handler))
        .route(
            "/auth/refresh",
            post(gateway_example::auth::refresh_handler),
        )
        .route("/auth/logout", post(gateway_example::auth::logout_handler))
        .route(
            "/auth/protected",
            get(gateway_example::auth::protected_handler).route_layer(
                middleware::from_fn_with_state(
                    state.clone(),
                    gateway_example::auth::auth_middleware,
                ),
            ),
        )
        .merge(service_router)
        .merge(gateway_example::openapi::swagger_ui_router::<GatewayState>())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
