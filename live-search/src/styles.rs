//! Scoped CSS classes for the live-search UI.
//!
//! `stylance::import_style!` imports deterministic hashed class names into Rust.
//! The matching stylesheet is generated separately by `stylance-cli` and served
//! by the Leptos shell as `/pkg/live-search.css`.
//!
//! The production/CI command is explicit about both crate and output:
//!
//! ```text
//! stylance live-search --output-file live-search/target/site/pkg/live-search.css
//! ```
//!
//! This module is compiled for both SSR and WASM targets, so the `stylance`
//! dependency remains in shared dependencies.

stylance::import_style!(pub css, "styles.module.css");

pub use css::*;
