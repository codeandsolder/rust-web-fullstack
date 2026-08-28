//! Unified application state — [`AppContext`] replacing individual `OnceLock`s.
//!
//! A single `Arc<AppContext>` is constructed at startup and stored both in a
//! global `OnceLock` (for server‑fn access) and provided via
//! `leptos::provide_context` (for the SSR component tree).
//!
//! # Backward compatibility
//!
//! The global [`set`] / [`get`] API exists for backward compatibility with
//! `#[server]` functions that do not take a context argument and therefore
//! cannot receive the context through the normal Leptos mechanism. **New
//! code should not use it.** New server functions should accept
//! `Arc<AppContext>` as a parameter and receive it via
//! `leptos::context::provide_context` (see [`crate::bootstrap`]).
//!
//! # Test seams
//!
//! Tests and the e2e launcher should construct an `AppContext` directly and
//! pass it where needed (or use the `test-seams` feature for the legacy
//! `db::set_pool` / `db::get_pool` globals).

use std::sync::Arc;
use std::sync::OnceLock;

use tokio::sync::broadcast;

use crate::cache::CacheHandle;
use crate::events::SseEvent;

/// Pool used by request-facing application queries.
///
/// The `otel` build wraps the raw `SQLx` pool with `sqlx-otel` so queries emit
/// OpenTelemetry database spans. Infrastructure that needs `SQLx`'s concrete
/// `PgPool` API (migrations and LISTEN/NOTIFY) keeps using the raw pool.
#[cfg(feature = "otel")]
pub type AppPool = sqlx_otel::Pool<sqlx::Postgres>;

/// Pool used by request-facing application queries when telemetry is disabled.
#[cfg(not(feature = "otel"))]
pub type AppPool = sqlx::PgPool;

/// Convert the raw `SQLx` pool into the request-facing application pool.
///
/// With `otel` enabled this wraps the same underlying pool handle in
/// `sqlx-otel`; without telemetry it is an identity conversion. Keeping this
/// construction in one place ensures production and integration tests exercise
/// the same instrumentation boundary.
#[cfg(feature = "otel")]
#[must_use]
pub fn app_pool_from_raw(raw_pool: sqlx::PgPool) -> AppPool {
    sqlx_otel::PoolBuilder::from(raw_pool).build()
}

/// Convert the raw `SQLx` pool into the request-facing application pool when
/// telemetry is disabled.
#[cfg(not(feature = "otel"))]
#[must_use]
pub const fn app_pool_from_raw(raw_pool: sqlx::PgPool) -> AppPool {
    raw_pool
}

/// Error returned when [`set`] is called more than once.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppContextInitError {
    /// The global context was already initialised.
    #[error("app context already initialized")]
    AlreadyInitialized,
}

/// Unified application state available to every subsystem.
///
/// Construct once at startup, then share via [`Arc`] (either through the
/// global [`set`] / [`get`] API or via `leptos::provide_context`).
pub struct AppContext {
    /// Database pool used by application queries.
    pub pool: AppPool,
    /// Broadcast sender for SSE events.
    pub broadcast: broadcast::Sender<SseEvent>,
    /// Search result cache.
    pub cache: CacheHandle,
}

impl AppContext {
    /// Create a new application context from its constituent parts.
    #[must_use]
    pub const fn new(
        pool: AppPool,
        broadcast: broadcast::Sender<SseEvent>,
        cache: CacheHandle,
    ) -> Self {
        Self {
            pool,
            broadcast,
            cache,
        }
    }
}

impl std::fmt::Debug for AppContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppContext")
            .field("pool", &self.pool)
            .field("broadcast", &self.broadcast)
            .field("cache", &self.cache)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Global storage (used by server functions and the leptos component tree)
// ---------------------------------------------------------------------------

static CONTEXT: OnceLock<Arc<AppContext>> = OnceLock::new();

/// Store the global application context.
///
/// Must be called exactly once during startup before any server function
/// or SSR request is processed.
///
/// # Errors
/// Returns [`AppContextInitError::AlreadyInitialized`] if called more than once.
pub fn set(ctx: Arc<AppContext>) -> Result<(), AppContextInitError> {
    CONTEXT
        .set(ctx)
        .map_err(|_| AppContextInitError::AlreadyInitialized)
}

/// Retrieve the global application context.
///
/// Returns `None` if [`set`] has not been called yet (defensive — callers
/// should check during startup and return an appropriate error).
#[must_use]
pub fn get() -> Option<&'static Arc<AppContext>> {
    CONTEXT.get()
}
