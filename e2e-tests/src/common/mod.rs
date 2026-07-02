//! Shared test helpers — split into focused submodules.
//!
//! Each submodule is `pub` so test files can `use e2e_tests::common::*`
//! directly. Items are public API; the rustc `dead_code` lint does not fire
//! on `pub` items in library crates, which is exactly why this module exists
//! at the crate root instead of inline under `tests/common/` (where each
//! binary compiled its own copy and forced per-function `#[allow(dead_code)]`
//! annotations).

pub mod chromium;
pub mod db;
pub mod gateway_env;
pub mod json;
pub mod live_search_env;
pub mod once;

// Convenience re-exports — flatten `common::chromium::setup` to
// `common::setup`, etc. so tests can do `use e2e_tests::common::{setup, ...}`.
pub use chromium::{
    element_is_visible, require_server, setup, teardown, wait_for_element, wait_for_js_true,
};
pub use db::TestEnv;
pub use gateway_env::GatewayEnv;
pub use live_search_env::LiveSearchEnv;
pub use once::SharedServer;
