//! `i18n-demo` — compile-time-checked internationalization with `leptos_i18n`.
//!
//! Locales: English (`en`) and German (`de`).  All translation keys are checked
//! at compile time via the `t!` macro.
//!
//! ## Feature gates
//!
//! - `ssr` — enables the Axum SSR server binary (the default build target).
//! - `hydrate` — enables the WASM hydration entry point.

pub mod app;
pub mod i18n;
pub mod styles;

// ---------------------------------------------------------------------------
// Hydrate entry point – called by the browser after the WASM module loads.
// ---------------------------------------------------------------------------

/// Entry point for WASM hydration.
///
/// Initialises the panic hook and hydrates the server-rendered HTML to make the
/// page interactive (attaches event handlers, starts the reactive system).
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
