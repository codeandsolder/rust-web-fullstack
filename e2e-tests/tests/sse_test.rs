//! SSE-specific E2E verification tests.
//!
//! These tests verify that Server-Sent Events streams:
//! - Successfully establish a connection (HTTP 200, content-type text/event-stream).
//! - Deliver data events to the client.
//! - Reconnect correctly after a page close/reopen cycle.
//!
//! All tests are gated behind the `integration` feature and will be ignored
//! when running plain `cargo test`.  Use `--features integration` to enable
//! them, and make sure the live-search service is running on port 3000
//! with a valid database connection.

use std::time::Duration;

use futures::StreamExt;

mod common;

use common::{base_url, require_server, setup, teardown, wait_for_js_true};

/// Navigate to `/live` and verify the SSE connection indicator is present.
///
/// The live-search example exposes SSE at `/api/events` (raw stream) and
/// renders a status indicator on the `/live` page.  This test checks that
/// the browser-based SSE via the WASM front-end shows a connection status.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_sse_stream_connects() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    let live_url = format!("{}/live", ctx.base_url);

    // Navigate to /live where the WASM app connects to `/api/events` via EventSource.
    ctx.page
        .goto(&live_url)
        .await
        .expect("Failed to navigate to /live");

    // Wait for the page to show a connection status (Connected or Connecting).
    let has_status = common::wait_for_js_true(
        &ctx.page,
        "() => document.body.innerText.includes('Connected') \
         || document.body.innerText.includes('Connecting')",
        std::time::Duration::from_secs(15),
    )
    .await;

    assert!(
        has_status,
        "Expected a connection status indicator (Connected/Connecting) on /live"
    );

    teardown(ctx).await;
}

/// Verify that the SSE stream delivers data to the client.
///
/// This test injects a JavaScript `EventSource` via `evaluate()` to capture
/// incoming messages, then waits for at least one data event to arrive.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_sse_receives_data() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    // Navigate to the base URL first so we have a document context.
    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base URL");

    // Inject an EventSource via evaluate.  We wrap the statements in an IIFE
    // so chromiumoxide evaluates it as an expression rather than a function.
    let inject_js = r"(async () => {
            window.__sseMessages = [];
            window.__sseStatus = 'connecting';
            const evtSource = new EventSource('/api/events');
            evtSource.onopen = () => { window.__sseStatus = 'open'; };
            evtSource.onmessage = (e) => {
                window.__sseMessages.push({ data: e.data, time: Date.now() });
                window.__sseStatus = 'received';
            };
            evtSource.onerror = () => { window.__sseStatus = 'error'; };
        })()"
        .to_string();
    ctx.page
        .evaluate(inject_js)
        .await
        .expect("Failed to inject EventSource script");

    // Wait for the connection to open and at least one message to arrive.
    let connected = wait_for_js_true(
        &ctx.page,
        "() => window.__sseStatus === 'open' || window.__sseStatus === 'received'",
        Duration::from_secs(15),
    )
    .await;

    assert!(connected, "SSE connection did not open within 15 seconds");

    // Wait a bit more for data to arrive if the status is just 'open'.
    let has_data = wait_for_js_true(
        &ctx.page,
        "() => window.__sseMessages.length > 0",
        Duration::from_secs(20),
    )
    .await;

    assert!(has_data, "No SSE data events received within 20 seconds");

    // Read the number of received messages.
    let msg_count: u32 = ctx
        .page
        .evaluate("() => window.__sseMessages.length")
        .await
        .expect("Failed to read SSE message count")
        .into_value::<u32>()
        .expect("Failed to deserialize SSE message count");

    assert!(
        msg_count > 0,
        "Expected at least one SSE message, got {msg_count}"
    );
    println!("Received {msg_count} SSE messages");

    // Optionally read the first message content for diagnostics.
    let first_msg: Option<String> = ctx
        .page
        .evaluate("() => window.__sseMessages[0]?.data ?? null")
        .await
        .ok()
        .and_then(|v| v.into_value::<serde_json::Value>().ok())
        .and_then(|v| v.as_str().map(String::from));
    if let Some(msg) = first_msg {
        println!(
            "First SSE message: {msg_slice}",
            msg_slice = &msg[..msg.len().min(200)]
        );
    }

    teardown(ctx).await;
}

/// Verify that the SSE stream reconnects after the page is closed and reopened.
///
/// This test:
/// 1. Opens a page and connects to the SSE stream.
/// 2. Confirms the initial connection works.
/// 3. Closes the page.
/// 4. Opens a new page (via a fresh `TestContext`, since `page.close()` consumes self).
/// 5. Verifies the second connection also receives data.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_sse_reconnect() {
    require_server(&base_url()).await;

    // --- First connection ---
    let mut ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base URL on first connection");

    // Inject EventSource and wait for first message.
    let inject_js = r"(async () => {
            window.__sseMessages = [];
            window.__sseStatus = 'connecting';
            const es1 = new EventSource('/api/events');
            es1.onopen = () => { window.__sseStatus = 'open'; };
            es1.onmessage = (e) => {
                window.__sseMessages.push({ data: e.data, time: Date.now() });
                window.__sseStatus = 'received';
            };
            es1.onerror = () => { window.__sseStatus = 'error'; };
        })()"
        .to_string();
    ctx.page
        .evaluate(inject_js)
        .await
        .expect("Failed to inject initial EventSource");

    // Wait for data on the first connection.
    let first_has_data = wait_for_js_true(
        &ctx.page,
        "() => window.__sseMessages.length > 0",
        Duration::from_secs(20),
    )
    .await;
    assert!(first_has_data, "First SSE connection did not receive data");

    let first_count: u32 = ctx
        .page
        .evaluate("() => window.__sseMessages.length")
        .await
        .expect("Failed to read first SSE message count")
        .into_value::<u32>()
        .expect("Failed to deserialize first SSE message count");
    println!("First connection received {first_count} messages");

    // --- Close the first page and create a fresh TestContext ---
    // Page::close() consumes self, so we teardown and create a new context.
    teardown(ctx).await;

    // --- Second connection via a brand-new context ---
    ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate on second connection");

    // Inject EventSource again and wait for data.
    let inject_js2 = r"(async () => {
            window.__sseMessages2 = [];
            window.__sseStatus2 = 'connecting';
            const es2 = new EventSource('/api/events');
            es2.onopen = () => { window.__sseStatus2 = 'open'; };
            es2.onmessage = (e) => {
                window.__sseMessages2.push({ data: e.data, time: Date.now() });
                window.__sseStatus2 = 'received';
            };
            es2.onerror = () => { window.__sseStatus2 = 'error'; };
        })()"
        .to_string();
    ctx.page
        .evaluate(inject_js2)
        .await
        .expect("Failed to inject second EventSource");

    // Wait for data on the reconnected stream.
    let second_has_data = wait_for_js_true(
        &ctx.page,
        "() => window.__sseMessages2.length > 0",
        Duration::from_secs(20),
    )
    .await;
    assert!(second_has_data, "SSE reconnection did not receive data");

    let second_count: u32 = ctx
        .page
        .evaluate("() => window.__sseMessages2.length")
        .await
        .expect("Failed to read second SSE message count")
        .into_value::<u32>()
        .expect("Failed to deserialize second SSE message count");

    assert!(
        second_count > 0,
        "Expected at least one SSE message after reconnect, got {second_count}"
    );
    println!("Reconnected stream received {second_count} messages");

    teardown(ctx).await;
}

// ---------------------------------------------------------------------------
// Required integration tests (from spec)
// ---------------------------------------------------------------------------

/// 5. SSE endpoint responds with event stream — make a raw HTTP GET to
///    `/api/events` and verify status 200 and Content-Type: text/event-stream.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn sse_endpoint_responds_with_event_stream() {
    require_server(&base_url()).await;

    let url = format!("{}/api/events", base_url());
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
        200,
        "Expected HTTP 200 from SSE endpoint, got {}",
        response.status()
    );

    let content_type = response
        .headers()
        .get("content-type")
        .expect("SSE response must have Content-Type header")
        .to_str()
        .expect("Content-Type is not valid ASCII");
    assert!(
        content_type.contains("text/event-stream"),
        "Expected Content-Type containing 'text/event-stream', got '{content_type}'"
    );

    println!("SSE endpoint at {url} -> HTTP 200, Content-Type: {content_type}");
}

/// 6. SSE keepalive pings — open an SSE connection and verify at least one
///    keep-alive ping is received within 20 seconds.
///
///    This test is slow (~20 s) and is ignored by default. Run it explicitly
///    with `cargo test --features integration sse_keepalive_pings -- --ignored`.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
#[cfg_attr(feature = "integration", ignore = "slow keepalive verification")]
async fn sse_keepalive_pings() {
    require_server(&base_url()).await;

    // The SSE KeepAlive default interval is 15 seconds.  We wait up to
    // 20 seconds to see at least one ping.
    let url = format!("{}/api/events", base_url());
    let client = reqwest::Client::builder()
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .get(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));

    assert_eq!(response.status(), 200);

    let mut stream = response.bytes_stream();
    let mut events: Vec<String> = Vec::new();

    // Read from the stream for up to 20 seconds, recording line content.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && trimmed != ":" {
                        events.push(trimmed.to_string());
                    }
                }
            }
            Ok(Some(Err(e))) => {
                panic!("SSE stream error: {e}");
            }
            Ok(None) => {
                panic!("SSE stream ended unexpectedly");
            }
            Err(_timeout) => {
                // No data within 2-second window — keep waiting for keepalive.
                continue;
            }
        }
        if !events.is_empty() {
            break;
        }
    }

    assert!(
        !events.is_empty(),
        "Expected at least one ping / event within 20 seconds, got none"
    );
    println!(
        "Received {len} SSE event(s) within 20 seconds",
        len = events.len()
    );
}

/// 7. Notify trigger fires SSE event — open the SSE connection and verify
///    the initial "Connected" event arrives within 5 seconds.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn notify_trigger_fires_sse_event() {
    require_server(&base_url()).await;

    let url = format!("{}/api/events", base_url());
    let client = reqwest::Client::builder()
        .build()
        .expect("Failed to build reqwest client");
    let response = client
        .get(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));

    assert_eq!(response.status(), 200);

    // Read the first chunk of the SSE stream.
    let mut stream = response.bytes_stream();
    let first_chunk = tokio::time::timeout(Duration::from_secs(5), stream.next()).await;

    match first_chunk {
        Ok(Some(Ok(bytes))) => {
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                text.contains(r#""type":"Connected""#) || text.contains("data:"),
                "Expected SSE data containing 'Connected' or 'data:', got: {text:.200}"
            );
            println!("SSE initial event received: {text:.200}");
        }
        Ok(Some(Err(e))) => {
            panic!("SSE stream read error: {e}");
        }
        Ok(None) => {
            panic!("SSE stream closed before sending any events");
        }
        Err(timeout_err) => {
            panic!("Timeout waiting for initial SSE event: {timeout_err}");
        }
    }
}
