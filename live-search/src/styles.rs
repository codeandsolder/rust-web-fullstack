//! Scoped CSS classes for the live-search UI.
//!
//! These are imported from `styles.module.css` via `stylance` at compile time
//! and injected into the page at runtime (SSR: as a `<style>` tag, WASM: via
//! the DOM). No `stylance-cli` build step is needed.
//!
//! # Feature notes
//!
//! This module is compiled for both SSR and WASM targets. The `stylance`
//! dependency is available in both configurations (listed in the crate's
//! shared `[dependencies]`).

stylance::import_style!(pub css, "styles.module.css");

// Re-export the class constants at the module root so callers can write
// `styles::nav` instead of `styles::css::nav`.
pub use css::*;
