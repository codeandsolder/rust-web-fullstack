//! Shared test helpers — split into focused submodules.
//!
//! Each `tests/*.rs` binary compiles its own copy of these helpers.

pub mod chromium;
pub mod db;
pub mod gateway_env;
pub mod json;
pub mod live_search_env;

// Re-export the most commonly used helpers so test files can
// `use common::*` or `use common::{setup, teardown, …}` directly.
// Each tests/*.rs binary compiles its own copy; unused exports are expected.
#[allow(unused_imports)]
pub use chromium::{
    element_is_visible, require_server, setup, teardown, wait_for_element, wait_for_js_true,
};
#[allow(unused_imports)]
pub use db::TestEnv;
#[allow(unused_imports)]
pub use gateway_env::GatewayEnv;
#[allow(unused_imports)]
pub use json::json_eq;
#[allow(unused_imports)]
pub use live_search_env::LiveSearchEnv;
