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

/// Login establishes both auth mechanisms, so `/auth/logout` must tear down
/// both the refresh-token family and the server-side cookie session.
#[tokio::test]
async fn auth_logout_revokes_refresh_state_and_flushes_session() -> Result<()> {
    let gateway = get_gateway().await?;
    let client = test_client()?;

    let login_response = client
        .post(format!("{}/auth/login", gateway.base_url()))
        .json(&serde_json::json!({
            "user_id": TEST_USER_ID,
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .context("failed to log in")?;
    assert_eq!(login_response.status(), reqwest::StatusCode::OK);

    let cookie = session_cookie(&login_response)?;
    let login: serde_json::Value = login_response
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

    let whoami_before = client
        .get(format!("{}/session/whoami", gateway.base_url()))
        .header(COOKIE, &cookie)
        .send()
        .await
        .context("failed to read session before logout")?;
    assert_eq!(whoami_before.status(), reqwest::StatusCode::OK);
    let whoami_before: serde_json::Value = whoami_before
        .json()
        .await
        .context("pre-logout whoami response is not valid JSON")?;
    assert_eq!(
        whoami_before.get("user_id").and_then(serde_json::Value::as_str),
        Some(TEST_USER_ID)
    );

    let csrf_response = client
        .get(format!("{}/auth/csrf", gateway.base_url()))
        .header(COOKIE, &cookie)
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
        .header(COOKIE, &cookie)
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .header(csrf_header, csrf_token)
        .send()
        .await
        .context("failed to call unified logout")?;
    assert_eq!(logout_response.status(), reqwest::StatusCode::OK);

    // Sending the old cookie explicitly must no longer recover session state;
    // this verifies server-side invalidation rather than relying on a browser
    // honoring the deletion Set-Cookie response.
    let whoami_after = client
        .get(format!("{}/session/whoami", gateway.base_url()))
        .header(COOKIE, &cookie)
        .send()
        .await
        .context("failed to read session after logout")?;
    assert_eq!(whoami_after.status(), reqwest::StatusCode::OK);
    let whoami_after: serde_json::Value = whoami_after
        .json()
        .await
        .context("post-logout whoami response is not valid JSON")?;
    assert!(
        whoami_after
            .get("user_id")
            .is_some_and(serde_json::Value::is_null),
        "session remained authenticated after /auth/logout: {whoami_after}"
    );

    let refresh_response = client
        .post(format!("{}/auth/refresh", gateway.base_url()))
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send()
        .await
        .context("failed to probe revoked refresh token")?;
    assert_eq!(refresh_response.status(), reqwest::StatusCode::UNAUTHORIZED);

    Ok(())
}
