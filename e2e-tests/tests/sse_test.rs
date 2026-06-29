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

use common::{base_url, require_server};

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
    require_server(&base_url(None)).await;

    let url = format!("{}/api/events", base_url(None));
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

/// 6. Notify trigger fires SSE event — open the SSE connection to `/api/events`,
///    insert a search result via `PostgreSQL`, then verify a `SearchResult` SSE
///    event containing the inserted title arrives within 10 seconds.
///
///    Requires `DATABASE_URL` to be set in the environment in addition to
///    `--features integration`.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration and DATABASE_URL"
)]
async fn notify_trigger_fires_sse_event() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");

    require_server(&base_url(None)).await;

    // ── 1. Open an SSE connection to /api/events ──────────────────────
    let url = format!("{}/api/events", base_url(None));
    let response = reqwest::Client::builder()
        .build()
        .expect("Failed to build reqwest client")
        .get(&url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to GET {url}: {e}"));

    assert_eq!(response.status(), 200);

    // ── 2. Insert a search result row via sqlx ────────────────────────
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    let title = format!(
        "e2e-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let test_url = "https://example.com/e2e-test";
    let snippet = "E2E test snippet for NOTIFY→SSE verification";

    // Clone for the INSERT — the original is needed for the assertion below.
    sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
        .bind(title.clone())
        .bind(test_url)
        .bind(snippet)
        .execute(&pool)
        .await
        .expect("Failed to insert search result");

    pool.close().await;

    // ── 3. Read SSE events until we see the SearchResult ──────────────
    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Timeout waiting for SearchResult SSE event. \
             Buffer so far (first 500 chars): {buf:.500}"
        );
        match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                buf.push('\n');
                if buf.contains(&title) && buf.contains("SearchResult") {
                    println!("SearchResult SSE event with title '{title}' received");
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("SSE stream error: {e}"),
            Ok(None) => {
                panic!("SSE stream ended before SearchResult. Buffer: {buf:.300}");
            }
            Err(_timeout) => {
                // No data in this 2 s window — keep waiting.
            }
        }
    }
}
