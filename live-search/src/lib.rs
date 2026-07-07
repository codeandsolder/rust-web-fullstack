//! Shared types and module declarations for the `live-search` crate.
//!
//! This crate provides a full-stack live-search application with:
//! - SSR server binary (axum + Leptos)
//! - WASM hydration client
//! - `PostgreSQL` full-text search with LISTEN/NOTIFY SSE streaming

pub mod app;
pub mod db;
pub mod events;
pub mod styles;

// `cache` is native-only (moka uses tokio) but does NOT depend on axum
// or leptos — gate on the wasm target so CI's `cargo test --workspace --lib`
// runs the cache unit tests even without `--features ssr`.
#[cfg(not(target_arch = "wasm32"))]
pub mod cache;
#[cfg(feature = "ssr")]
pub mod sse;

#[cfg(feature = "ssr")]
pub mod bootstrap;
#[cfg(feature = "ssr")]
pub mod shutdown;

#[cfg(feature = "otel")]
pub mod otel;

// ---------------------------------------------------------------------------
// Hydrate entry point – called by the browser after the WASM module loads.
// ---------------------------------------------------------------------------

/// Entry point for WASM hydration. Called by the browser after the module
/// initialises, it hydrates the server-rendered HTML to make the page
/// interactive (attaches event handlers, starts reactive system, etc.).
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
