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

use common::{base_url, require_server, setup, teardown};

/// Navigate to `/health` and verify the response is 200 + valid JSON.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_gateway_health() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    let health_url = format!("{}/health", ctx.base_url);

    // Navigate to the health endpoint.  `goto()` resolves after the page is
    // fully loaded.  Chromiumoxide does not expose HTTP status via goto(),
    // so we verify by checking the page content below.
    ctx.page
        .goto(&health_url)
        .await
        .expect("Failed to navigate to /health");

    // The body should be parseable JSON.
    let body: String = ctx
        .page
        .content()
        .await
        .expect("Failed to read page content");

    // Strip any HTML wrapper — a /health endpoint that returns JSON directly
    // would not wrap it.  If the body contains HTML we try to extract text
    // inside a <pre> or <body>.
    let json_str = if body.trim_start().starts_with('<') {
        // Try extracting text from <pre> or <body> via evaluate.
        let extracted: String = ctx
            .page
            .evaluate("document.body?.innerText ?? ''")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .unwrap_or_else(|| body.clone());
        extracted.trim().to_string()
    } else {
        body.trim().to_string()
    };

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("Response body is not valid JSON");
    println!("Health endpoint response: {parsed}");

    // At minimum the JSON should be an object (not an array or primitive).
    assert!(
        parsed.is_object(),
        "Health response should be a JSON object"
    );

    teardown(ctx).await;
}

/// Navigate to `/` and verify a JSON list of services is returned.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_gateway_service_list() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base URL");

    let body: String = ctx
        .page
        .content()
        .await
        .expect("Failed to read page content");
    let extracted: String = ctx
        .page
        .evaluate("document.body?.innerText ?? ''")
        .await
        .ok()
        .and_then(|v| v.into_value::<String>().ok())
        .unwrap_or(body);
    let parsed: serde_json::Value =
        serde_json::from_str(extracted.trim()).expect("Response body is not valid JSON");
    let services = parsed
        .get("services")
        .and_then(|v| v.as_array())
        .expect("Gateway root response should contain a services array");
    assert!(
        !services.is_empty(),
        "Gateway should advertise at least one service"
    );
    println!("Service list: {parsed}");

    teardown(ctx).await;
}

/// For each discovered service, verify its health route is reachable (HTTP 200).
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_gateway_service_routes_accessible() {
    require_server(&base_url()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");

    // GET / to discover services.
    let response = client
        .get(base_url())
        .send()
        .await
        .expect("Failed to GET /");
    let json: serde_json::Value = response.json().await.expect("Response is not valid JSON");
    let services = json["services"].as_array().expect("services array");

    assert!(
        !services.is_empty(),
        "Gateway should advertise at least one service"
    );

    // Verify each service route is reachable.
    for service in services {
        let path = service["path"].as_str().expect("service has path");
        let url = format!("{}/{path}/health", base_url());
        let resp = client
            .get(&url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));
        let status = resp.status();
        assert!(
            status.is_success(),
            "Service health route {path}/health returned unexpected status {status}"
        );
        println!("  /{path}/health -> {status}");
    }
}

/// POST to /auth/login and verify a JWT token is returned in the response.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_gateway_login_endpoint() {
    require_server(&base_url()).await;
    let url = format!("{}/auth/login", base_url());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .post(&url)
        .json(&serde_json::json!({"user_id": "admin", "password": "admin"}))
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to POST {url}: {e}"));

    let status = response.status();
    println!("Login endpoint returned {status}");
    assert_eq!(status, 200, "Expected default admin login to succeed");

    let json: serde_json::Value = response.json().await.expect("Response is not valid JSON");
    let token = json["token"]
        .as_str()
        .expect("Response should have a 'token' field");
    assert_eq!(
        token.split('.').count(),
        3,
        "JWT should have 3 dot-separated segments"
    );
    println!(
        "Successfully received JWT token ({len} chars)",
        len = token.len()
    );
}

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
    require_server(&base_url()).await;

    let url = base_url();
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
    require_server(&base_url()).await;

    let url = base_url();
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
    require_server(&base_url()).await;

    let url = format!("{}/health", base_url());
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
    require_server(&base_url()).await;

    let url = format!("{}/auth/login", base_url());
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
    require_server(&base_url()).await;

    let url = format!("{}/auth/protected", base_url());
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
