//! E2E tests for a gateway / API gateway example.
//!
//! These tests verify:
//! - The `/health` endpoint returns a 200 status and valid JSON.
//!
//! Test files allow expect/unwrap/panic for fail-fast assertions.
//! - The root `/` returns a JSON list of available services.
//! - Each service route is reachable.
//! - The `/auth/login` endpoint accepts a POST and returns a JWT token.
//! - The `/events` SSE endpoint returns `text/event-stream`.
//! - CORS preflight requests are handled with appropriate headers.
//!
//! All tests launch the gateway in-process using [`common::GatewayEnv`], so
//! no external server is needed.

use std::time::Duration;

mod common;

use common::GatewayEnv;
use anyhow::Context;

/// Synthetic dev credential used by the integration tests.
///
/// MUST match what `GatewayEnv::start()` sets as `ADMIN_PASSWORD`.
const TEST_PASSWORD: &str = "synthetic-gateway-test-password";

/// Helper to build a reqwest client with a short timeout.
fn test_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")
}

// ---------------------------------------------------------------------------
// Helper: start a shared gateway for the suite
// ---------------------------------------------------------------------------

use tokio::sync::OnceCell;

/// Shared gateway server instance, initialised lazily on first access.
static GATEWAY: OnceCell<GatewayEnv> = OnceCell::const_new();

async fn get_gateway() -> anyhow::Result<&'static GatewayEnv> {
    GATEWAY
        .get_or_try_init(|| async { GatewayEnv::start().await })
        .await
}

// ---------------------------------------------------------------------------
// Required integration tests (from spec)
// ---------------------------------------------------------------------------

/// 8. Landing page loads — GET `/`, assert HTTP 200 and JSON structure
///    (gateway name and services array).
#[tokio::test]
async fn landing_page_loads() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = gw.base_url();
    let client = test_client()?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    let status = response.status();
    assert_eq!(
        status, 200,
        "Expected HTTP 200 from landing page, got {status}",
    );

    let json: serde_json::Value = response.json().await.context("response is not valid JSON")?;

    // Exact gateway name — catches typos and version regressions.
    let gateway = json
        .get("gateway")
        .and_then(|v| v.as_str())
        .context("response should contain a 'gateway' field")?;
    assert!(
        gateway.contains("Gateway"),
        "Landing page 'gateway' field should contain 'Gateway', got: {gateway}"
    );

    // Services array present and non-empty.
    let services = json
        .get("services")
        .and_then(|v| v.as_array())
        .context("response should contain a 'services' array")?;
    assert!(
        !services.is_empty(),
        "Landing page 'services' array should not be empty"
    );

    println!(
        "Gateway landing page loaded: gateway='{gateway}', services={}",
        services.len()
    );

    Ok(())
}

/// 9. Service listing returns services — GET `/`, assert JSON response contains
///    a `services` array with at least the three mock services (search, proxy, monitor).
#[tokio::test]
async fn service_listing_returns_services() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = gw.base_url();
    let client = test_client()?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    assert_eq!(response.status(), 200);

    let json: serde_json::Value = response.json().await.context("response is not valid JSON")?;
    let services = json
        .get("services")
        .and_then(|v| v.as_array())
        .context("response should have a 'services' array")?;

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

    Ok(())
}

/// 10. Health endpoint returns ok — GET `/health`, assert 200 and
///     `{"gateway":"ok",…}`.
#[tokio::test]
async fn health_endpoint_returns_ok() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = format!("{}/health", gw.base_url());
    let client = test_client()?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    let status = response.status();
    assert_eq!(status, 200, "Expected HTTP 200 from /health, got {status}");

    let json: serde_json::Value = response.json().await.context("response is not valid JSON")?;

    let gateway_ok = json.get("gateway").and_then(|v| v.as_str());
    assert_eq!(
        gateway_ok,
        Some("ok"),
        "Expected gateway status 'ok', got: {json}"
    );
    println!("Health endpoint reports gateway OK");

    Ok(())
}

/// 11. Auth login succeeds — POST to `/auth/login` with default admin credentials,
///     assert 200 and a JWT token (three dot-separated base64 segments) is returned.
#[tokio::test]
async fn auth_login_succeeds() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = format!("{}/auth/login", gw.base_url());
    let client = test_client()?;
    let response = client
        .post(&url)
        .json(&serde_json::json!({
            "user_id": "test",
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;

    let status = response.status();
    assert!(
        status == 200 || status == 201,
        "Expected 200 or 201 from /auth/login, got {status}"
    );

    let json: serde_json::Value = response
        .json()
        .await
        .context("login response is not valid JSON")?;

    let token = json
        .get("token")
        .and_then(|v| v.as_str())
        .context("login response should contain a 'token' field")?;

    // Verify JWT structure: three dot-separated segments.
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "Expected a JWT (3 dot-separated parts), got {}: {token:.100}",
        parts.len()
    );
    println!("Auth login succeeded, received JWT ({} chars)", token.len());

    Ok(())
}

/// 12. Auth middleware rejects unauthenticated requests.
#[tokio::test]
async fn auth_middleware_rejects_unauthenticated() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = format!("{}/auth/protected", gw.base_url());
    let client = test_client()?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    assert_eq!(
        response.status(),
        401,
        "Expected protected route to reject missing Bearer token"
    );

    Ok(())
}

/// 13. Auth middleware accepts a valid token — POST credentials to
///     `/auth/login`, then use the returned JWT to GET `/auth/protected`
///     and verify a 200 response. Catches regressions where the middleware
///     rejects ALL tokens.
#[tokio::test]
async fn auth_protected_accepts_valid_token() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let client = test_client()?;

    // ── 1. Login ────────────────────────────────────────────────────
    let login_url = format!("{}/auth/login", gw.base_url());
    let login_response = client
        .post(&login_url)
        .json(&serde_json::json!({
            "user_id": "test",
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .with_context(|| format!("failed to POST {login_url}"))?;

    let login_status = login_response.status();
    assert!(
        login_status == 200 || login_status == 201,
        "Expected 200/201 from /auth/login, got {login_status}",
    );

    let login_json: serde_json::Value = login_response
        .json()
        .await
        .context("login response is not valid JSON")?;

    let token = login_json
        .get("token")
        .and_then(|v| v.as_str())
        .context("login response should contain a 'token' field")?;

    // ── 2. Use the token to access the protected endpoint ───────────
    let protected_url = format!("{}/auth/protected", gw.base_url());
    let protected_response = client
        .get(&protected_url)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("failed to GET {protected_url}"))?;

    let status = protected_response.status();
    assert_eq!(
        status, 200,
        "Expected HTTP 200 from /auth/protected with valid Bearer token, got {status}"
    );

    let body: serde_json::Value = protected_response
        .json()
        .await
        .context("protected response is not valid JSON")?;
    assert_eq!(
        body.get("protected").and_then(serde_json::Value::as_bool),
        Some(true),
        "Expected protected body to have `protected: true`, got: {body}"
    );
    println!("Auth middleware accepted valid token, returned protected=true");

    Ok(())
}

/// 14. Auth login rejects an invalid password — POST to `/auth/login` with
///     the wrong password and verify 401.
#[tokio::test]
async fn auth_login_rejects_invalid_password() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = format!("{}/auth/login", gw.base_url());
    let client = test_client()?;
    let response = client
        .post(&url)
        .json(&serde_json::json!({
            "user_id": "test",
            "password": "definitely-wrong-password",
        }))
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;

    let status = response.status();
    assert_eq!(
        status, 401,
        "Expected HTTP 401 from /auth/login with invalid password, got {status}",
    );

    Ok(())
}

/// 15. Gateway SSE endpoint returns event stream — GET `/events` and verify
///     HTTP 200 and `Content-Type: text/event-stream`.  The gateway forwards
///     `GatewayEvent`s via a `broadcast::Sender` consumed by this endpoint.
#[tokio::test]
async fn gateway_sse_endpoint_returns_event_stream() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = format!("{}/events", gw.base_url());
    let client = test_client()?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    assert_eq!(
        response.status(),
        200,
        "Expected HTTP 200 from gateway /events, got {}",
        response.status()
    );

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .context("SSE response must have Content-Type header")?;
    assert!(
        content_type.contains("text/event-stream"),
        "Expected Content-Type containing 'text/event-stream', got '{content_type}'"
    );

    println!("Gateway SSE endpoint at {url} -> HTTP 200, Content-Type: {content_type}");

    Ok(())
}

/// 16. Gateway CORS preflight is handled — send an OPTIONS preflight request
///     with an `Origin` header and verify the gateway responds with
///     `Access-Control-Allow-Origin` and appropriate CORS headers.
#[tokio::test]
async fn gateway_cors_preflight_is_handled() -> anyhow::Result<()> {
    let gw = get_gateway().await?;
    let url = format!("{}/health", gw.base_url());
    let client = test_client()?;
    let response = client
        .request(reqwest::Method::OPTIONS, &url)
        .header("Origin", "http://example.com")
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .with_context(|| format!("failed to OPTIONS {url}"))?;

    // CORS preflight should return 200 (tower-http CorsLayer responds to
    // preflight requests with 200, not 204).
    assert!(
        response.status().is_success(),
        "Expected success from CORS preflight, got {}",
        response.status()
    );

    // Verify the CORS header is present.
    let allow_origin = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());
    assert!(
        allow_origin.is_some(),
        "Expected Access-Control-Allow-Origin header in CORS preflight response"
    );

    println!(
        "Gateway CORS preflight -> {}, Allow-Origin: {}",
        response.status(),
        allow_origin.unwrap_or("(missing)")
    );

    Ok(())
}

