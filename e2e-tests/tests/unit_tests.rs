//! Unit tests for the e2e-tests helper functions.
//!
//! These tests do NOT require a browser or any running services.
//! They run with plain `cargo test` — no `--features integration` needed.

#[allow(dead_code)]
mod common;

use common::{base_url, join_url};

// ---------------------------------------------------------------------------
// base_url()
// ---------------------------------------------------------------------------

#[test]
fn test_base_url_default() {
    // Passing None reads from env. In test runs without BASE_URL set, the
    // fallback is "http://localhost:3000".  We just verify it is a valid URL.
    let url = base_url(None);
    assert!(!url.is_empty(), "base_url(None) should not be empty");
    assert!(
        url.starts_with("http://") || url.starts_with("https://"),
        "base_url(None) should return a valid HTTP URL, got: {url}"
    );
}

#[test]
fn test_base_url_explicit_override() {
    // Passing Some(...) bypasses the environment entirely — no unsafe needed.
    let url = base_url(Some("http://example.com:8080"));
    assert_eq!(url, "http://example.com:8080");
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
