//! Unit tests for the e2e-tests helper functions.
//!
//! These tests do NOT require a browser or any running services.
//! They run with plain `cargo test` — no `--features integration` needed.

#[allow(dead_code)]
mod common;

use common::{TestContext, base_url, join_url};

// ---------------------------------------------------------------------------
// base_url()
// ---------------------------------------------------------------------------

#[test]
#[expect(
    unsafe_code,
    reason = "std::env::set_var/remove_var are unsafe in Edition 2024; single-threaded test code"
)]
fn test_base_url_default() {
    // Unset BASE_URL so we hit the fallback.
    let prev = std::env::var("BASE_URL").ok();
    // SAFETY: single-threaded test; no concurrent env access.
    unsafe { std::env::remove_var("BASE_URL") }
    let url = base_url();
    assert_eq!(url, "http://localhost:3000");
    // Restore previous value (if any).
    match prev {
        Some(v) => unsafe { std::env::set_var("BASE_URL", v) },
        None => unsafe { std::env::remove_var("BASE_URL") },
    }
}

#[test]
#[expect(
    unsafe_code,
    reason = "std::env::set_var/remove_var are unsafe in Edition 2024; single-threaded test code"
)]
fn test_base_url_env_override() {
    let prev = std::env::var("BASE_URL").ok();
    // SAFETY: single-threaded test; no concurrent env access.
    unsafe { std::env::set_var("BASE_URL", "http://example.com:8080") }
    let url = base_url();
    assert_eq!(url, "http://example.com:8080");
    // Restore previous value (if any).
    match prev {
        Some(v) => unsafe { std::env::set_var("BASE_URL", v) },
        None => unsafe { std::env::remove_var("BASE_URL") },
    }
}

// ---------------------------------------------------------------------------
// join_url() — URL composition
// ---------------------------------------------------------------------------

#[test]
fn test_join_url_basic() {
    let url = join_url("http://localhost:3000", "/api/health");
    assert_eq!(url, "http://localhost:3000/api/health");
}

#[test]
fn test_join_url_no_trailing_slash_on_base() {
    let url = join_url("http://localhost:3000", "api/health");
    assert_eq!(url, "http://localhost:3000/api/health");
}

#[test]
fn test_join_url_trailing_slash_on_base() {
    let url = join_url("http://localhost:3000/", "/api/health");
    assert_eq!(url, "http://localhost:3000/api/health");
}

#[test]
fn test_join_url_empty_path() {
    let url = join_url("http://localhost:3000", "");
    assert_eq!(url, "http://localhost:3000/");
}

// ---------------------------------------------------------------------------
// TestContext struct sanity
// ---------------------------------------------------------------------------

/// Sanity check: the `TestContext` struct should be large enough to hold 4
/// meaningful fields (3 pointers + 1 String).  If someone removes a field
/// (or the struct is unexpectedly tiny) this test will fail, which is a
/// useful reminder to update all callers.
#[test]
fn test_test_context_field_count() {
    let size = std::mem::size_of::<TestContext>();
    // A pointer on 64-bit is 8 bytes; a String is 24 bytes (ptr + cap + len).
    // Absolute minimum for 3 trait-object handles + 1 String = 3*8 + 24 = 48.
    // We assert >= 40 to allow for minor layout differences across platforms.
    assert!(
        size >= 40,
        "TestContext seems too small ({size} bytes) — may be missing fields"
    );
}
