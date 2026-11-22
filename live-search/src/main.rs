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

use anyhow::Context;
use live_search::{bootstrap, shutdown};
use rwf_config::{Config, Server, StartupOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let options = StartupOptions::parse(Server::LiveSearch, std::env::args_os().skip(1))?;
    let cfg =
        Config::load(options.config_path.as_deref()).context("failed to load workspace config")?;
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let mut handle = bootstrap::run(&cfg.live_search, &database_url).await?;
    shutdown::wait(handle.shutdown, &mut handle.tasks, &handle.pool).await
}
