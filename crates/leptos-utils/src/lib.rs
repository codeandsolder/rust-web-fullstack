//! # Deprecated
//!
//! This crate was created as a workaround for the "doubled-prefix" probe:
//! `#[server(endpoint = "/api/…")]` declarations under Leptos's default
//! `prefix = "/api"` registered the function at `/api/api/…` while the wire
//! client called `/api/…`, so [`probed_server_fn_handler`] rewrote the
//! request URI to the doubled path on every request.
//!
//! That was a configuration error, not a Leptos bug: the fix is to declare
//! `endpoint = "…"` (relative to the default `/api` prefix). The probe is
//! therefore no longer needed and has been removed.
//!
//! The crate remains in the workspace only as a re-export shim so existing
//! callers of [`probed_server_fn_handler`] keep compiling; it now simply
//! re-exports [`leptos_axum::handle_server_fns`].

/// Deprecated alias for [`leptos_axum::handle_server_fns`].
///
/// Kept so callers that mounted the old doubled-prefix probe keep compiling;
/// new code should call [`leptos_axum::handle_server_fns`] directly.
pub use leptos_axum::handle_server_fns;
