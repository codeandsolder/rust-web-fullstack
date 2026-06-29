//! E2E tests for a gateway / API gateway example.
//!
//! These tests verify:
//! - The `/health` endpoint returns a 200 status and valid JSON.
//! - The root `/` returns a JSON list of available services.
//! - Each service route is reachable.
//! - The `/auth/login` endpoint accepts a POST and returns a JWT token.
//!
//! All tests are gated behind the `integration` feature and will be ignored
//! when running plain `cargo test`.  Use `--features integration` to enable
//! them, and make sure the gateway service is running on port 3001
//! (set `BASE_URL=http://localhost:3001` in the environment).

use std::time::Duration;

mod common;

use common::require_server;
use e2e_tests::base_url;

// ---------------------------------------------------------------------------
// Required integration tests (from spec)
// ---------------------------------------------------------------------------

/// 8. Landing page loads — GET `/`, assert HTTP 200 and body mentions "Gateway".
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn landing_page_loads() {
    require_server(&base_url(None)).await;

    let url = base_url(None);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .get(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));

    let status = response.status();
    assert_eq!(
        status, 200,
        "Expected HTTP 200 from landing page, got {status}",
    );

    let body = response.text().await.expect("Failed to read response body");
    assert!(
        body.contains("Gateway"),
        "Landing page body should contain 'Gateway', got: {body:.200}"
    );
    println!(
        "Gateway landing page loaded (body length: {len})",
        len = body.len()
    );
}

/// 9. Service listing returns services — GET `/`, assert JSON response contains
///    a `services` array with at least the three mock services (search, proxy, monitor).
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn service_listing_returns_services() {
    require_server(&base_url(None)).await;

    let url = base_url(None);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .get(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));

    assert_eq!(response.status(), 200);

    let json: serde_json::Value = response.json().await.expect("Response is not valid JSON");
    let services = json
        .get("services")
        .and_then(|v| v.as_array())
        .expect("Response should have a 'services' array");

    assert!(!services.is_empty(), "Services array should not be empty");

    let names: Vec<&str> = services
        .iter()
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()))
        .collect();

    for expected in &["search", "proxy", "monitor"] {
        assert!(
            names.contains(expected),
            "Services should include '{expected}', got: {names:?}"
        );
    }
    println!(
        "Service listing returned {len} services: {names:?}",
        len = names.len()
    );
}

/// 10. Health endpoint returns ok — GET `/health`, assert 200 and
///     `{"gateway":"ok",…}`.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn health_endpoint_returns_ok() {
    require_server(&base_url(None)).await;

    let url = format!("{}/health", base_url(None));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .get(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));

    let status = response.status();
    assert_eq!(status, 200, "Expected HTTP 200 from /health, got {status}");

    let json: serde_json::Value = response.json().await.expect("Response is not valid JSON");

    let gateway_ok = json.get("gateway").and_then(|v| v.as_str());
    assert_eq!(
        gateway_ok,
        Some("ok"),
        "Expected gateway status 'ok', got: {json}"
    );
    println!("Health endpoint reports gateway OK");
}

/// 11. Auth login succeeds — POST to `/auth/login` with default admin credentials,
///     assert 200 and a JWT token (three dot-separated base64 segments) is returned.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn auth_login_succeeds() {
    require_server(&base_url(None)).await;

    let url = format!("{}/auth/login", base_url(None));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .post(&url)
        .json(&serde_json::json!({"user_id": "test", "password": "admin"}))
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to POST {url}: {e}"));

    let status = response.status();
    assert!(
        status == 200 || status == 201,
        "Expected 200 or 201 from /auth/login, got {status}"
    );

    let json: serde_json::Value = response
        .json()
        .await
        .expect("Login response is not valid JSON");

    let token = json
        .get("token")
        .and_then(|v| v.as_str())
        .expect("Login response should contain a 'token' field");

    // Verify JWT structure: three dot-separated segments.
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "Expected a JWT (3 dot-separated parts), got {}: {token:.100}",
        parts.len()
    );
    println!("Auth login succeeded, received JWT ({} chars)", token.len());
}

/// 12. Auth middleware rejects unauthenticated requests.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn auth_middleware_rejects_unauthenticated() {
    require_server(&base_url(None)).await;

    let url = format!("{}/auth/protected", base_url(None));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .get(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));

    assert_eq!(
        response.status(),
        401,
        "Expected protected route to reject missing Bearer token"
    );
}
