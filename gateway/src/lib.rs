//! Gateway example — a composable service gateway built with Axum.
//!
//! This crate provides an extensible gateway pattern: each service
//! implements [`module::ServiceModule`] and is mounted under its own path
//! prefix. The gateway aggregates health checks (via `join_all`), provides
//! SSE event streaming, enforces JWT-based authentication with `EdDSA`
//! signatures, and loads configuration from environment variables.
//!
//! # Architecture
//!
//! * [`gateway`] — [`GatewayState`], [`build_gateway`], health / root
//!   handlers
//! * [`auth`] — JWT creation / validation (`EdDSA`), auth middleware,
//!   login / refresh / logout / protected handlers
//! * [`module`] — [`ServiceModule`] trait, [`ServiceInfo`],
//!   [`ServiceHealthError`]
//! * [`services`] — concrete [`ServiceModule`] implementations with typed
//!   DTOs
//! * [`settings`] — environment-based configuration with redacted [`Debug`]
//!   and `--dev-keys` ephemeral keypair generation
//! * [`sse`] — Server-Sent Events via [`broadcast::Sender`]
//! * [`openapi`] — `OpenAPI` schema generation via `utoipa`
//! * [`otel`] — OpenTelemetry tracing setup (behind `otel` feature)

pub mod auth;
pub mod gateway;
pub mod module;
pub mod openapi;
pub mod pem;
pub mod services;
pub mod settings;
pub mod sse;

#[cfg(feature = "otel")]
pub mod otel;
