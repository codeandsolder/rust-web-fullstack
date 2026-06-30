//! Verification test for the in-process gateway launcher.
//!
//! This test:
//! - Spins up Postgres via `TestEnv::postgres()` (to prove it works with DB,
//!   though the gateway itself doesn't need one).
//! - Calls `GatewayEnv::start()` to launch the gateway in-process.
//! - Asserts the returned address is reachable.
//! - Asserts `GET {addr}/health` returns 200.
//!
//! Pre-fix (no launcher): this test would not compile ("function not found").
//! Post-fix: this test passes.

use std::time::Duration;

mod common;

use anyhow::Context as _;
use common::{GatewayEnv, TestEnv};

/// Verify that `GatewayEnv::start()` launches a reachable gateway that
/// responds to health checks.
#[tokio::test]
async fn gateway_launcher_creates_reachable_server() -> anyhow::Result<()> {
    // ── 1. Spin up a Postgres testcontainer (proves DB infra works) ──
    let _db = TestEnv::postgres().await?;

    // ── 2. Launch the in-process gateway ─────────────────────────────
    let gw = GatewayEnv::start().await?;

    // ── 3. The returned address must be a valid socket address ────────
    let addr = gw.addr();
    assert!(addr.port() > 0, "Gateway must bind to a non-zero port");
    println!("Gateway bound to {addr}");

    // ── 4. The /health endpoint must return 200 ──────────────────────
    let url = format!("http://{addr}/health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("gateway at {url} is not reachable"))?;

    let status = response.status();
    if status != 200 {
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read body: {e}>"));
        return Err(anyhow::anyhow!(
            "Expected HTTP 200 from gateway /health, got {status}.\nResponse body:\n{body_text}"
        ));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .context("health response is not valid JSON")?;

    let gateway_ok = body.get("gateway").and_then(|v| v.as_str());
    assert_eq!(
        gateway_ok,
        Some("ok"),
        "Expected gateway status 'ok', got: {body}"
    );

    println!("Gateway health check returned gateway=ok");

    // ── 5. Clean shutdown ────────────────────────────────────────────
    gw.shutdown().await;

    Ok(())
}
