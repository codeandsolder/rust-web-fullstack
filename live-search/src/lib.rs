//! Shared types and module declarations for the `live-search` crate.
//!
//! This crate provides a full-stack live-search application with:
//! - SSR server binary (axum + Leptos)
//! - WASM hydration client
//! - `PostgreSQL` full-text search with LISTEN/NOTIFY SSE streaming

// `TableRow` 0.19/derive 0.15 generates async trait methods whose bodies are
// immediately ready. Clippy 1.98's `unused_async_trait_impl` fires in that
// generated code. Keep this scoped to the module that invokes the derive, and
// make it an expectation so a future macro upgrade that fixes the expansion
// turns this suppression stale and CI asks us to remove it.
#[expect(
    clippy::unused_async_trait_impl,
    reason = "leptos-struct-table TableRow derive generates immediately-ready async trait methods"
)]
pub mod app;
pub mod db;
pub mod events;
pub mod styles;

// `state` depends on sqlx + tokio (native-only) and is currently only used
// under `feature = "ssr"`.
#[cfg(feature = "ssr")]
pub mod state;

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
// Hydrate entry point – called by Leptos HydrationScripts after WASM init.
// ---------------------------------------------------------------------------

/// Entry point for WASM hydration. Called by Leptos after the module
/// initialises, it hydrates the server-rendered HTML to make the page
/// interactive (attaches event handlers, starts reactive system, etc.).
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
