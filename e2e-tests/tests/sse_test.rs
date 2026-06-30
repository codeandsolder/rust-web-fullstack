//! SSE-specific E2E verification tests.
//!
//! These tests verify that Server-Sent Events streams:
//! - Successfully establish a connection (HTTP 200, content-type text/event-stream).
//! - Deliver data events to the client.
//! - Reconnect correctly after a page close/reopen cycle.
//!
//! All tests use an in-process live-search server backed by a testcontainer
//! Postgres database, so no external services are required.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Once;
use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;
use tokio::sync::OnceCell;

mod common;

use common::LiveSearchEnv;

/// Shared live-search server instance, initialised lazily on first access.
///
/// Wraps a `Result` so that initialisation failures are propagated to every
/// caller without panicking inside the background thread.
static LIVE_SEARCH: OnceCell<Result<LiveSearchEnv, Arc<anyhow::Error>>> = OnceCell::const_new();
static BG_INIT_DONE: AtomicBool = AtomicBool::new(false);
static BG_INIT_ONCE: Once = Once::new();

/// Get the shared server instance, running [`LiveSearchEnv::start()`] on a
/// persistent background tokio runtime so the server's database pool is not
/// tied to any single test runtime.
async fn get_server() -> anyhow::Result<&'static LiveSearchEnv> {
    BG_INIT_ONCE.call_once(|| {
        // IIFE so we can use `?` inside a `call_once` closure (which returns `()`).
        let result: Result<(), Arc<anyhow::Error>> = (|| {
            let handle = std::thread::Builder::new()
                .name("e2e-bg-init".into())
                .spawn(move || {
                    let rt = match tokio::runtime::Runtime::new() {
                        Ok(rt) => rt,
                        Err(e) => {
                            let err = Arc::new(anyhow::Error::new(e).context(
                                "failed to create background init runtime",
                            ));
                            let _ = LIVE_SEARCH.set(Err(err));
                            BG_INIT_DONE.store(true, Ordering::Release);
                            return;
                        }
                    };
                    rt.block_on(async {
                        match LiveSearchEnv::start().await {
                            Ok(env) => {
                                let _ = LIVE_SEARCH.set(Ok(env));
                            }
                            Err(e) => {
                                let _ = LIVE_SEARCH.set(Err(Arc::new(e)));
                            }
                        }
                        BG_INIT_DONE.store(true, Ordering::Release);
                        if LIVE_SEARCH.get().is_none_or(Result::is_ok) {
                            // Keep the runtime alive indefinitely so the pools'
                            // background management tasks survive individual test
                            // runtimes.
                            std::future::pending::<()>().await;
                        }
                    });
                })
                .map_err(|e| {
                    Arc::new(anyhow::Error::new(e).context("failed to spawn background init thread"))
                })?;
            let _ = handle;
            Ok(())
        })();

        // If we couldn't even spawn the thread, surface the failure via
        // LIVE_SEARCH so the test-side loop sees the error instead of timing out.
        if let Err(err) = result {
            let _ = LIVE_SEARCH.set(Err(err));
            BG_INIT_DONE.store(true, Ordering::Release);
        }
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !BG_INIT_DONE.load(Ordering::Acquire) {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "background LiveSearchEnv initialization timed out after 30s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let cell_ref = LIVE_SEARCH
        .get()
        .context("LiveSearchEnv not initialized")?;
    cell_ref
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{e:#}"))
}

/// Helper: build a reqwest client with a short timeout.
fn test_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")
}

// ---------------------------------------------------------------------------
// Required integration tests (from spec)
// ---------------------------------------------------------------------------

/// 5. SSE endpoint responds with event stream — make a raw HTTP GET to
///    `/api/events` and verify status 200 and Content-Type: text/event-stream.
#[tokio::test]
async fn sse_endpoint_responds_with_event_stream() -> anyhow::Result<()> {
    let env = get_server().await?;
    let url = format!("{}/api/events", env.base_url());
    let client = test_client()?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    assert_eq!(
        response.status(),
        200,
        "Expected HTTP 200 from SSE endpoint, got {}",
        response.status()
    );

    let content_type = response
        .headers()
        .get("content-type")
        .context("SSE response must have Content-Type header")?
        .to_str()
        .context("content-type is not valid ASCII")?;
    assert!(
        content_type.contains("text/event-stream"),
        "Expected Content-Type containing 'text/event-stream', got '{content_type}'"
    );

    println!("SSE endpoint at {url} -> HTTP 200, Content-Type: {content_type}");
    Ok(())
}

/// 6. Notify trigger fires SSE event — open the SSE connection to `/api/events`,
///    insert a search result via `PostgreSQL`, then verify a `SearchResult` SSE
///    event containing the inserted title arrives within 10 seconds.
///
///    Uses the in-process live-search server with a testcontainer Postgres DB,
///    so no external services are required.
#[tokio::test]
async fn notify_trigger_fires_sse_event() -> anyhow::Result<()> {
    let env = get_server().await?;
    let conn_str = env.db().connection_string().to_string();
    let pool = sqlx::PgPool::connect(&conn_str)
        .await
        .with_context(|| format!("failed to connect to {conn_str}"))?;

    // ── 0. Pre-clean: wipe any leftover e2e-test rows from previous failed runs.
    sqlx::query("DELETE FROM search_results WHERE title LIKE 'e2e-%' OR title LIKE 'e2e-warmup-%' OR title LIKE 'browser-sse-sentinel-%'")
        .execute(&pool)
        .await
        .ok();

    // ── 1. Open an SSE connection to /api/events ──────────────────────
    let client = test_client()?;
    let url = format!("{}/api/events", env.base_url());
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    assert_eq!(response.status(), 200);
    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    // ── 2. Insert a warmup row and wait for it to appear in the stream ─
    //
    // PostgreSQL NOTIFY is best-effort: if `PgListener` hasn't yet called
    // `LISTEN search_results` (cold start, slow CI runner), the NOTIFY is
    // dropped silently and the assertion below would time out for the wrong
    // reason. We can't observe `PgListener::listen()` directly, but we CAN
    // prove the listener is active by inserting a sentinel row and waiting
    // for the resulting SSE event. (Waiting for `SseEvent::Connected` is
    // NOT sufficient — that event is emitted by the SSE handler itself,
    // independent of the broadcast/listener pipeline.)
    let warmup_title = format!(
        "e2e-warmup-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let warmup_url = format!("https://example.com/e2e-warmup/{warmup_title}");
    sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
        .bind(&warmup_title)
        .bind(&warmup_url)
        .bind("SSE warmup row to prove PgListener is LISTEN-ing")
        .execute(&pool)
        .await
        .context("failed to insert warmup row")?;

    let warmup_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            tokio::time::Instant::now() < warmup_deadline,
            "Timeout waiting for warmup SSE event — PgListener may not be \
             LISTEN-ing yet. Buffer so far (first 500 chars): {buf:.500}"
        );
        match tokio::time::timeout(Duration::from_secs(1), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                buf.push('\n');
                if buf.contains(&warmup_title) && buf.contains("SearchResult") {
                    break;
                }
            }
            Ok(Some(Err(e))) => return Err(anyhow::anyhow!("SSE stream error during warmup: {e}")),
            Ok(None) => return Err(anyhow::anyhow!("SSE stream ended during warmup. Buffer: {buf:.300}")),
            Err(_timeout) => {} // keep waiting
        }
    }
    println!("Warmup SSE event received — PgListener is LISTEN-ing");

    // ── 3. Insert the real test row ──────────────────────────────────
    let title = format!(
        "e2e-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    // Use a unique URL per test run so the UNIQUE constraint on `url` does
    // not reject the INSERT when the test is re-run.
    let test_url = format!("https://example.com/e2e-test/{title}");
    let snippet = "E2E test snippet for NOTIFY→SSE verification";

    sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
        .bind(&title)
        .bind(&test_url)
        .bind(snippet)
        .execute(&pool)
        .await
        .context("failed to insert search result")?;

    // ── 4. Read SSE events until we see the SearchResult ──────────────
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

                    // Snapshot the event payload for regression detection.
                    // Fields like `title`, `url`, `snippet` contain the
                    // inserted values — the snapshot captures the shape.
                    if let Some(event_json) = parse_sse_json_payload(&buf, &title) {
                        // unwrap is safe: to_string_pretty only fails on
                        // non-finite floats, which serde_json::Value from a
                        // known schema never contains.
                        event_snapshot(event_json);
                    }

                    // Best-effort cleanup so re-runs don't accumulate rows.
                    if let Err(e) = sqlx::query("DELETE FROM search_results WHERE title = $1")
                        .bind(&title)
                        .execute(&pool)
                        .await
                    {
                        eprintln!("warning: failed to delete e2e-test row '{title}': {e}");
                    }
                    if let Err(e) = sqlx::query("DELETE FROM search_results WHERE title = $1")
                        .bind(&warmup_title)
                        .execute(&pool)
                        .await
                    {
                        eprintln!("warning: failed to delete e2e-warmup row '{warmup_title}': {e}");
                    }
                    return Ok(());
                }
            }
            Ok(Some(Err(e))) => return Err(anyhow::anyhow!("SSE stream error: {e}")),
            Ok(None) => {
                return Err(anyhow::anyhow!(
                    "SSE stream ended before SearchResult. Buffer: {buf:.300}"
                ));
            }
            Err(_timeout) => {
                // No data in this 2 s window — keep waiting.
            }
        }
    }
}

/// Try to extract the SSE event JSON for the test row from the accumulated
/// buffer.  Returns `None` if the payload hasn't fully arrived yet.
///
/// The SSE stream delivers text/event-stream chunks; the JSON payload lives
/// after `data: ` lines.
fn parse_sse_json_payload(buf: &str, title: &str) -> Option<serde_json::Value> {
    // Find the line containing the title after "data: "
    for line in buf.lines() {
        if let Some(payload) = line.strip_prefix("data: ")
            && payload.contains(title)
        {
            return serde_json::from_str(payload).ok();
        }
    }
    None
}

/// Replace dynamic test identifiers with stable placeholders so the snapshot
/// is deterministic across runs, then snapshot the result.
fn event_snapshot(mut value: serde_json::Value) {
    if let Some(val) = value.get_mut("title")
        && let Some(title) = val.as_str()
        && (title.starts_with("e2e-test-") || title.starts_with("e2e-warmup-"))
    {
        *val = serde_json::Value::String("<DYNAMIC_TEST_TITLE>".to_string());
    }
    if let Some(val) = value.get_mut("url")
        && let Some(url) = val.as_str()
        && (url.contains("e2e-test-") || url.contains("e2e-warmup-"))
    {
        *val = serde_json::Value::String("<DYNAMIC_TEST_URL>".to_string());
    }
    insta::assert_json_snapshot!("sse_search_result_event", value);
}
