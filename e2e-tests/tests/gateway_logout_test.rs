//! Regression coverage for the gateway's unified logout semantics.

use std::time::Duration;

use anyhow::{Context, Result};
use e2e_tests::common::GatewayEnv;
use reqwest::header::{AUTHORIZATION, COOKIE, SET_COOKIE};

use e2e_tests::common::once::SharedServer;

const TEST_PASSWORD: &str = "synthetic-gateway-test-password";
const TEST_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

static GATEWAY: SharedServer<GatewayEnv> = SharedServer::new();

async fn get_gateway() -> Result<&'static GatewayEnv> {
    GATEWAY.get(|| async { GatewayEnv::start().await }).await
}

fn test_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")
}

fn session_cookie(response: &reqwest::Response) -> Result<String> {
    let prefix = format!("{}=", gateway_example::session::SESSION_COOKIE_NAME);

    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .find(|cookie| cookie.starts_with(&prefix))
        .map(str::to_owned)
        .context("login response did not set the gateway session cookie")
}

async fn login(client: &reqwest::Client, gateway: &GatewayEnv) -> Result<(String, String, String)> {
    let response = client
        .post(format!("{}/auth/login", gateway.base_url()))
        .json(&serde_json::json!({
            "user_id": TEST_USER_ID,
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .context("failed to log in")?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let cookie = session_cookie(&response)?;
    let login: serde_json::Value = response
        .json()
        .await
        .context("login response is not valid JSON")?;
    let access_token = login
        .get("token")
        .and_then(serde_json::Value::as_str)
        .context("login response is missing access token")?
        .to_owned();
    let refresh_token = login
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .context("login response is missing refresh token")?
        .to_owned();

    Ok((cookie, access_token, refresh_token))
}

async fn whoami(
    client: &reqwest::Client,
    gateway: &GatewayEnv,
    cookie: &str,
) -> Result<serde_json::Value> {
    client
        .get(format!("{}/session/whoami", gateway.base_url()))
        .header(COOKIE, cookie)
        .send()
        .await
        .context("failed to read session")?
        .error_for_status()
        .context("session whoami failed")?
        .json()
        .await
        .context("whoami response is not valid JSON")
}

async fn assert_refresh_revoked(
    client: &reqwest::Client,
    gateway: &GatewayEnv,
    refresh_token: &str,
) -> Result<()> {
    let response = client
        .post(format!("{}/auth/refresh", gateway.base_url()))
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send()
        .await
        .context("failed to probe revoked refresh token")?;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    Ok(())
}

/// `/auth/logout` revokes every outstanding refresh token for the authenticated
/// subject, but it can only flush the cookie session attached to this request.
#[tokio::test]
async fn auth_logout_revokes_subject_refresh_state_and_flushes_only_current_session() -> Result<()>
{
    let gateway = get_gateway().await?;
    let client = test_client()?;

    // Establish two independent browser-like sessions for the same subject.
    let (cookie_a, access_a, refresh_a) = login(&client, gateway).await?;
    let (cookie_b, _access_b, refresh_b) = login(&client, gateway).await?;
    assert_ne!(cookie_a, cookie_b, "independent logins reused a session ID");

    let current_session_before = whoami(&client, gateway, &cookie_a).await?;
    let other_session_before = whoami(&client, gateway, &cookie_b).await?;
    assert_eq!(
        current_session_before
            .get("user_id")
            .and_then(serde_json::Value::as_str),
        Some(TEST_USER_ID)
    );
    assert_eq!(
        other_session_before
            .get("user_id")
            .and_then(serde_json::Value::as_str),
        Some(TEST_USER_ID)
    );

    let csrf_response = client
        .get(format!("{}/auth/csrf", gateway.base_url()))
        .header(COOKIE, &cookie_a)
        .send()
        .await
        .context("failed to bootstrap CSRF token")?;
    assert_eq!(csrf_response.status(), reqwest::StatusCode::OK);
    let csrf: serde_json::Value = csrf_response
        .json()
        .await
        .context("CSRF response is not valid JSON")?;
    let csrf_token = csrf
        .get("csrf_token")
        .and_then(serde_json::Value::as_str)
        .context("CSRF response is missing csrf_token")?;
    let csrf_header = csrf
        .get("header")
        .and_then(serde_json::Value::as_str)
        .context("CSRF response is missing header name")?;

    let logout_response = client
        .post(format!("{}/auth/logout", gateway.base_url()))
        .header(COOKIE, &cookie_a)
        .header(AUTHORIZATION, format!("Bearer {access_a}"))
        .header(csrf_header, csrf_token)
        .send()
        .await
        .context("failed to call unified logout")?;
    assert_eq!(logout_response.status(), reqwest::StatusCode::OK);

    // The exact cookie used for logout must be dead server-side, not merely
    // deleted by Set-Cookie on a cooperative browser.
    let current_session_after = whoami(&client, gateway, &cookie_a).await?;
    assert!(
        current_session_after
            .get("user_id")
            .is_some_and(serde_json::Value::is_null),
        "logout session remained authenticated: {current_session_after}"
    );

    // The handler cannot enumerate other MemoryStore sessions for the subject,
    // so a different cookie session remains authenticated. This is deliberate
    // current behavior and is distinct from refresh-token revocation scope.
    let other_session_after = whoami(&client, gateway, &cookie_b).await?;
    assert_eq!(
        other_session_after
            .get("user_id")
            .and_then(serde_json::Value::as_str),
        Some(TEST_USER_ID),
        "logout unexpectedly flushed another cookie session"
    );

    // Refresh revocation is subject-wide, so credentials from both logins die.
    assert_refresh_revoked(&client, gateway, &refresh_a).await?;
    assert_refresh_revoked(&client, gateway, &refresh_b).await?;

    Ok(())
}
