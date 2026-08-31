//! Session-security regressions for the gateway.

use std::time::Duration;

use anyhow::{Context, ensure};
use e2e_tests::common::{GatewayEnv, SharedServer};
use reqwest::header::{COOKIE, SET_COOKIE};

const TEST_PASSWORD: &str = "synthetic-gateway-test-password";
const TEST_USER_ID: &str = "00000000-0000-0000-0000-000000000001";
const SESSION_COOKIE: &str = "rwf_session";

static GATEWAY: SharedServer<GatewayEnv> = SharedServer::new();

async fn get_gateway() -> anyhow::Result<&'static GatewayEnv> {
    GATEWAY.get(|| async { GatewayEnv::start().await }).await
}

fn test_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")
}

fn response_cookie(response: &reqwest::Response, name: &str) -> anyhow::Result<String> {
    for header in response.headers().get_all(SET_COOKIE) {
        let header = header.to_str().context("Set-Cookie header was not UTF-8")?;
        let Some((cookie_name, rest)) = header.split_once('=') else {
            continue;
        };
        if cookie_name.trim() != name {
            continue;
        }

        let value = rest.split(';').next().unwrap_or_default();
        ensure!(!value.is_empty(), "{name} cookie had an empty value");
        return Ok(value.to_owned());
    }

    anyhow::bail!("response did not set the {name} cookie")
}

#[tokio::test]
async fn login_rotates_pre_auth_session_id_and_invalidates_the_old_cookie() -> anyhow::Result<()> {
    let gateway = get_gateway().await?;
    let base = gateway.base_url();
    let client = test_client()?;

    // CSRF bootstrap deliberately creates useful pre-authentication session
    // state. Capture that cookie as if another party already knew its ID.
    let csrf_response = client
        .get(format!("{base}/auth/csrf"))
        .send()
        .await
        .context("failed to bootstrap CSRF session")?
        .error_for_status()
        .context("CSRF bootstrap failed")?;
    let old_session = response_cookie(&csrf_response, SESSION_COOKIE)?;

    // Authenticate while presenting the known pre-authentication session ID.
    let login_response = client
        .post(format!("{base}/auth/login"))
        .header(COOKIE, format!("{SESSION_COOKIE}={old_session}"))
        .json(&serde_json::json!({
            "user_id": TEST_USER_ID,
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .context("failed to login with pre-authentication session")?
        .error_for_status()
        .context("login failed")?;
    let new_session = response_cookie(&login_response, SESSION_COOKIE)?;

    ensure!(
        old_session != new_session,
        "login retained the pre-authentication session ID"
    );

    // The exact old credential must no longer resolve to authenticated state.
    let stale_whoami: serde_json::Value = client
        .get(format!("{base}/session/whoami"))
        .header(COOKIE, format!("{SESSION_COOKIE}={old_session}"))
        .send()
        .await
        .context("failed to query stale session")?
        .error_for_status()
        .context("stale session whoami failed")?
        .json()
        .await
        .context("stale session whoami returned invalid JSON")?;
    ensure!(
        stale_whoami["user_id"].is_null(),
        "old pre-authentication session inherited authenticated state: {stale_whoami}"
    );

    // The rotated credential must still carry the authenticated session state.
    let active_whoami: serde_json::Value = client
        .get(format!("{base}/session/whoami"))
        .header(COOKIE, format!("{SESSION_COOKIE}={new_session}"))
        .send()
        .await
        .context("failed to query rotated session")?
        .error_for_status()
        .context("rotated session whoami failed")?
        .json()
        .await
        .context("rotated session whoami returned invalid JSON")?;
    ensure!(
        active_whoami["user_id"] == TEST_USER_ID,
        "rotated session did not retain authenticated state: {active_whoami}"
    );

    Ok(())
}

#[tokio::test]
async fn replaying_rotated_refresh_token_revokes_the_replacement_family() -> anyhow::Result<()> {
    let gateway = get_gateway().await?;
    let base = gateway.base_url();
    let client = test_client()?;

    let login: serde_json::Value = client
        .post(format!("{base}/auth/login"))
        .json(&serde_json::json!({
            "user_id": TEST_USER_ID,
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .context("failed to login before refresh rotation")?
        .error_for_status()
        .context("login before refresh rotation failed")?
        .json()
        .await
        .context("login response returned invalid JSON")?;
    let old_refresh = login["refresh_token"]
        .as_str()
        .context("login response did not contain a refresh token")?
        .to_owned();

    let rotation: serde_json::Value = client
        .post(format!("{base}/auth/refresh"))
        .json(&serde_json::json!({ "refresh_token": old_refresh }))
        .send()
        .await
        .context("failed to rotate refresh token")?
        .error_for_status()
        .context("refresh-token rotation failed")?
        .json()
        .await
        .context("refresh-token rotation returned invalid JSON")?;
    let new_refresh = rotation["refresh_token"]
        .as_str()
        .context("refresh-token rotation did not return a replacement")?
        .to_owned();

    // Reusing the exact old capability is the replay signal. It must fail and
    // revoke every still-active credential in that token family.
    let replay = client
        .post(format!("{base}/auth/refresh"))
        .json(&serde_json::json!({ "refresh_token": old_refresh }))
        .send()
        .await
        .context("failed to replay rotated refresh token")?;
    ensure!(
        replay.status() == reqwest::StatusCode::UNAUTHORIZED,
        "replayed refresh token was accepted: {}",
        replay.status()
    );

    let replacement_after_replay = client
        .post(format!("{base}/auth/refresh"))
        .json(&serde_json::json!({ "refresh_token": new_refresh }))
        .send()
        .await
        .context("failed to probe replacement after replay")?;
    ensure!(
        replacement_after_replay.status() == reqwest::StatusCode::UNAUTHORIZED,
        "refresh-token replay did not revoke the replacement family: {}",
        replacement_after_replay.status()
    );

    Ok(())
}
