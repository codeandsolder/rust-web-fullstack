//! Service module trait and metadata types.
//!
//! Defines [`ServiceModule`] (the trait every mountable service implements),
//! [`ServiceInfo`] (read-only metadata for API discovery), and
//! [`ServiceHealthError`] (the error type for health-check failures).

use axum::Router;
use futures::future::BoxFuture;

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
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[error("service unavailable: {reason}")]
#[must_use = "a ServiceHealthError must be observed; consider logging or returning it to the caller"]
pub struct ServiceHealthError {
    pub reason: String,
}

/// A composable service module that can be mounted under the gateway.
pub trait ServiceModule: Send + Sync {
    /// Short unique identifier (used for logs / events).
    fn name(&self) -> &'static str;

    /// URL path prefix under which this service is mounted.
    fn path(&self) -> &'static str {
        self.name()
    }

    /// Human-readable summary for the service listing endpoint.
    fn description(&self) -> &'static str;

    /// Whether the service is active. Disabled modules are not mounted.
    fn enabled(&self) -> bool {
        true
    }

    /// The axum Router whose handlers all share [`GatewayState`].
    fn router(&self) -> Router<GatewayState>;

    /// Lightweight health probe. Every module must implement this explicitly;
    /// there is deliberately no unconditional-green default.
    #[must_use = "a health check result should be observed or returned to the caller"]
    fn health_check(&self) -> BoxFuture<'_, Result<(), ServiceHealthError>>;
}
