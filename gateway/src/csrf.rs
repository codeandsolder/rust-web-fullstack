//! CSRF protection via `axum-tower-sessions-csrf`.
//!
//! The CSRF middleware is layered on top of the session manager and validates
//! a synchronizer token on every mutating request (`POST`, `PUT`, `DELETE`,
//! `PATCH`).  Clients fetch the token via [`get_or_create_token`] and include
//! it in the [`TOKEN_HEADER`] request header.

/// Re-export the CSRF middleware type.
pub use axum_tower_sessions_csrf::CsrfMiddleware;

/// Re-export the token endpoint helper.
pub use axum_tower_sessions_csrf::get_or_create_token;

/// Re-export the HTTP header name clients must use.
pub use axum_tower_sessions_csrf::TOKEN_HEADER;
