//! Service module trait and metadata types.
//!
//! Defines [`ServiceModule`] (the trait every mountable service implements),
//! [`ServiceInfo`] (read-only metadata for API discovery), and
//! [`ServiceHealthError`] (the error type for health-check failures).

use axum::Router;
use futures::future::{self, BoxFuture, FutureExt};

use crate::gateway::GatewayState;

/// Describes a registered service for API discovery / nav rendering.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ServiceInfo {
    pub name: &'static str,
    pub path: &'static str,
    pub description: &'static str,
    pub enabled: bool,
}

/// Error returned by service module health checks.
///
/// Intentionally string-based because the gateway has no opinion about which
/// underlying error type (sqlx, redis, http, …) a particular service depends
/// on.  Services that need to preserve the full error chain should wrap their
/// concrete error via [`anyhow::Error::new`] or a custom `#[source] source`
/// field when this type is specialised in the future.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[error("service unavailable: {reason}")]
#[must_use = "a ServiceHealthError must be observed; consider logging or returning it to the caller"]
pub struct ServiceHealthError {
    /// Human-readable reason for the failed health check.
    pub reason: String,
}

/// A composable service module that can be mounted under the gateway.
///
/// Each implementation provides its own axum `Router` scoped under
/// the path returned by [`ServiceModule::path`].  The gateway handles
/// lifecycle, health aggregation, and SSE event forwarding.
pub trait ServiceModule: Send + Sync {
    /// Short unique identifier (used for logs / events).
    fn name(&self) -> &'static str;

    /// URL path prefix under which this service is mounted.
    /// Defaults to [`ServiceModule::name`].
    fn path(&self) -> &'static str {
        self.name()
    }

    /// Human-readable summary for the service listing endpoint.
    fn description(&self) -> &'static str;

    /// Whether the service is active.  Disabled modules are not mounted.
    fn enabled(&self) -> bool {
        true
    }

    /// The axum Router whose handlers all share [`GatewayState`].
    fn router(&self) -> Router<GatewayState>;

    /// Lightweight health probe.  Return `Ok(())` if the service is healthy.
    ///
    /// The trait is object-safe so modules can be stored as `dyn ServiceModule`.
    /// Returning an explicit boxed future keeps the allocation visible instead
    /// of hiding it behind an async-trait macro.
    #[must_use = "a health check result should be observed or returned to the caller"]
    fn health_check(&self) -> BoxFuture<'_, Result<(), ServiceHealthError>> {
        future::ready(Ok(())).boxed()
    }
}
