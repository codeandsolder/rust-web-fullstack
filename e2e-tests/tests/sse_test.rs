//! SSE-specific E2E verification tests.
//!
//! These tests verify that Server-Sent Events streams:
//! - Successfully establish a connection (HTTP 200, content-type text/event-stream).
//! - Deliver data events to the client.
//! - Reconnect correctly after a page close/reopen cycle.
//!
//! All tests use an in-process live-search server backed by a testcontainer
//! Postgres database, so no external services are required.

use std::time::Duration;

use anyhow::Context;
use e2e_tests::common::{LiveSearchEnv, SharedServer};
use futures::{Stream, StreamExt};

/// Shared live-search server instance, initialised lazily on first access.
static SERVER: SharedServer<LiveSearchEnv> = SharedServer::new();

/// Get the shared server instance, running [`LiveSearchEnv::start()`] on a
/// persistent background tokio runtime so the server's database pool is not
/// tied to any single test runtime.
async fn get_server() -> anyhow::Result<&'static LiveSearchEnv> {
    SERVER.get(|| async { LiveSearchEnv::start().await }).await
}

/// Helper: build a reqwest client with a short timeout.
fn test_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")
}

fn unique_test_title(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    )
}

async fn insert_search_result(
    pool: &sqlx::PgPool,
    title: &str,
    url: &str,
    snippet: &str,
    context: &'static str,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
        .bind(title)
        .bind(url)
        .bind(snippet)
        .execute(pool)
        .await
        .context(context)?;
    Ok(())
}

async fn wait_for_search_result<S, B, E>(
    stream: &mut S,
    buffer: &mut String,
    title: &str,
    phase: &str,
    poll_timeout: Duration,
) -> anyhow::Result<()>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "Timeout waiting for {phase} SSE event. Buffer so far (first 500 chars): {buffer:.500}"
            );
        }

        match tokio::time::timeout(poll_timeout, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buffer.push_str(&String::from_utf8_lossy(chunk.as_ref()));
                buffer.push('\n');
                if buffer.contains(title) && buffer.contains("SearchResult") {
                    return Ok(());
                }
            }
            Ok(Some(Err(error))) => anyhow::bail!("SSE stream error during {phase}: {error}"),
            Ok(None) => {
                anyhow::bail!("SSE stream ended during {phase}. Buffer: {buffer:.300}");
            }
            Err(_) => {}
        }
    }
}

async fn cleanup_test_row(pool: &sqlx::PgPool, title: &str, label: &str) {
    if let Err(error) = sqlx::query("DELETE FROM search_results WHERE title = $1")
        .bind(title)
        .execute(pool)
        .await
    {
        eprintln!("warning: failed to delete {label} row '{title}': {error}");
    }
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

    sqlx::query("DELETE FROM search_results WHERE title LIKE 'e2e-%' OR title LIKE 'e2e-warmup-%' OR title LIKE 'browser-sse-sentinel-%'")
        .execute(&pool)
        .await
        .ok();

    let client = test_client()?;
    let url = format!("{}/api/events", env.base_url());
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;
    assert_eq!(response.status(), 200);

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    // PostgreSQL NOTIFY is best-effort, so first prove PgListener has reached
    // LISTEN before inserting the row whose event the test actually asserts.
    let warmup_title = unique_test_title("e2e-warmup");
    let warmup_url = format!("https://example.com/e2e-warmup/{warmup_title}");
    insert_search_result(
        &pool,
        &warmup_title,
        &warmup_url,
        "SSE warmup row to prove PgListener is LISTEN-ing",
        "failed to insert warmup row",
    )
    .await?;
    wait_for_search_result(
        &mut stream,
        &mut buffer,
        &warmup_title,
        "warmup",
        Duration::from_secs(1),
    )
    .await?;
    println!("Warmup SSE event received — PgListener is LISTEN-ing");

    let title = unique_test_title("e2e-test");
    let test_url = format!("https://example.com/e2e-test/{title}");
    insert_search_result(
        &pool,
        &title,
        &test_url,
        "E2E test snippet for NOTIFY→SSE verification",
        "failed to insert search result",
    )
    .await?;
    wait_for_search_result(
        &mut stream,
        &mut buffer,
        &title,
        "SearchResult",
        Duration::from_secs(2),
    )
    .await?;
    println!("SearchResult SSE event with title '{title}' received");

    if let Some(event_json) = parse_sse_json_payload(&buffer, &title) {
        event_snapshot(event_json);
    }

    cleanup_test_row(&pool, &title, "e2e-test").await;
    cleanup_test_row(&pool, &warmup_title, "e2e-warmup").await;
    Ok(())
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
