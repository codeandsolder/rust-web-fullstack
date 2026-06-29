//! Gateway example — a composable service gateway built with Axum.
//!
//! This crate provides an extensible gateway pattern: each service
//! implements [`module::ServiceModule`] and is mounted under its own path
//! prefix. The gateway aggregates health checks (via `join_all`), provides
//! SSE event streaming, enforces JWT-based authentication with constant-time
//! password comparison and per-IP rate limiting, and loads configuration from
//! environment variables.
//!
//! # Architecture
//!
//! * [`gateway`] — [`GatewayState`], [`build_gateway`], health / root /
//!   protected handlers
//! * [`auth`] — JWT creation / validation, login handler, auth middleware,
//!   rate limiter
//! * [`module`] — [`ServiceModule`] trait, [`ServiceInfo`],
//!   [`ServiceHealthError`]
//! * [`services`] — concrete [`ServiceModule`] implementations
//! * [`settings`] — environment-based configuration with redacted [`Debug`]
//! * [`sse`] — Server-Sent Events via [`broadcast::Sender`]

pub mod auth;
pub mod gateway;
pub mod module;
pub mod services;
pub mod settings;
pub mod sse;
