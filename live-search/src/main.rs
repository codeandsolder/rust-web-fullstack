//! SSR server binary entry point for the live-search application.
//!
//! A thin launcher that delegates to [`bootstrap::run`] and then
//! [`shutdown::wait`]. See those modules for the full setup / teardown
//! logic.
//!
//! # Feature notes
//!
//! - `otel` feature: enables OpenTelemetry tracing + `/metrics` endpoint.
//! - `dev-tools` feature (requires `RUSTFLAGS="--cfg tokio_unstable"`):
//!   enables `console-subscriber` for Tokio task inspection. Baked into
//!   the dev Docker image (Phase 2).

#![cfg(feature = "ssr")]

use live_search::{bootstrap, shutdown};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut handle = bootstrap::run().await?;
    shutdown::wait(handle.shutdown, &mut handle.tasks, &handle.pool).await
}
