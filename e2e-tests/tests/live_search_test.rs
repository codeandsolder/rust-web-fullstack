//! E2E tests for a live-search example.
//!
//! These tests verify:
//! - The server function catch-all (Pattern 9) routes `/api/search` correctly.
//! - Static WASM/JS assets are served from `/pkg/` (Critical Rule 7).
//! - The root path is reachable (SSR HTML or in-process fallback).
//! - Unknown paths return 404 via the fallback handler.
//! - Browser tests (behind `--features browser-tests`): search input present,
//!   search shows results, nonsense query shows "no results" message,
//!   SSE live-feed indicator appears on `/live`.
//!
//! HTTP-level tests use an in-process live-search server backed by a
//! testcontainer Postgres database.  Browser-level tests (behind
//! `--features browser-tests`) additionally require a Chromium installation.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;

#[cfg(feature = "browser-tests")]
use chromiumoxide::Page;
#[cfg(feature = "browser-tests")]
use e2e_tests::common::{element_is_visible, setup, teardown, wait_for_element, wait_for_js_true};

use e2e_tests::common::{LiveSearchEnv, SharedServer};

/// Shared live-search server instance, initialised lazily on first access.
static SERVER: SharedServer<LiveSearchEnv> = SharedServer::new();

/// Get the shared server instance, running [`LiveSearchEnv::start()`] on a
/// persistent background tokio runtime so the server's database pool is not
/// tied to any single test runtime.
async fn get_server() -> anyhow::Result<&'static LiveSearchEnv> {
    SERVER.get(|| async { LiveSearchEnv::start().await }).await
}

// ---------------------------------------------------------------------------
// Browser-based tests (require `--features browser-tests` + Chrome/Chromium)
// ---------------------------------------------------------------------------

#[cfg(feature = "browser-tests")]
mod browser_tests {
    use super::*;

    /// Fill the search input by setting its value and dispatching a single
    /// `input` event. Leptos 0.8's `bind:value` listens for the `input` event
    /// to update the underlying signal; `WebElement::type_str` types characters
    /// one at a time without firing the right input event on every Keystroke,
    /// so we set the value directly and dispatch a single bubbleable event.
    async fn fill_search_input(page: &Page, query: &str) -> anyhow::Result<()> {
        // Wait for hydration: Leptos adds `data-hk` attributes to elements it
        // hydrates. Leptos 0.8.x does not expose a hydration marker attribute on
        // arbitrary elements (`data-hk` is only attached by the view macro inside
        // keyed list iteration), so we use a short fixed delay instead. The local
        // server hydrates in tens of milliseconds, so 750 ms is generous.
        tokio::time::sleep(Duration::from_millis(750)).await;

        // The chromiumoxide `Page::evaluate` API detects arrow-function syntax and
        // routes to `Runtime.callFunctionOn` with the function declaration but
        // **no arguments** — so any parameter is `undefined` at runtime. That makes
        // the standard `el.value = q` pattern silently set the value to `undefined`
        // (coerced to `""`), the input event fires, and `bind:value` never sees the
        // query we typed. Fix: inline the value into an IIFE so the function takes
        // no parameters and the value is part of the function body itself.
        let value_json = serde_json::to_string(query).context("query is always valid JSON")?;
        let script = format!(
            r#"(() => {{
                const el = document.querySelector('[data-testid="search-input"]');
                if (!el) throw new Error('search input not found');
                el.focus();
                const setter = Object.getOwnPropertyDescriptor(
                    window.HTMLInputElement.prototype, 'value'
                ).set;
                setter.call(el, {value_json});
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                return el.value;
            }})()"#
        );
        let echoed: String = page
            .evaluate(script.as_str())
            .await
            .context("failed to set search input value")?
            .into_value::<String>()
            .context("failed to deserialize echoed value")?;
        anyhow::ensure!(
            echoed == query,
            "value was not set correctly: browser reports '{echoed}', wanted '{query}'"
        );
        Ok(())
    }

    /// 2. Search returns results — type a query, submit, wait for result items.
    ///    Asserts at least one `[data-testid="result-item"]` appears and its
    ///    text contains the query substring.
    #[tokio::test]
    async fn search_returns_results() -> anyhow::Result<()> {
        let env = get_server().await?;
        let ctx = setup().await?;

        ctx.page
            .goto(env.base_url())
            .await
            .context("failed to navigate to homepage")?;

        // Wait for the input to be present.
        let _search_input = wait_for_element(
            &ctx.page,
            r#"[data-testid="search-input"]"#,
            Duration::from_secs(5),
        )
        .await
        .context("search input not found")?;

        // Set the search query (must dispatch `input` event for Leptos `bind:value`).
        let query = "rust";
        fill_search_input(&ctx.page, query).await?;

        // Click the submit button.
        ctx.page
            .find_element(r#"[data-testid="search-submit"]"#)
            .await
            .context("search button not found")?
            .click()
            .await
            .context("failed to click search button")?;

        let has_results = wait_for_js_true(
            &ctx.page,
            "() => document.querySelectorAll('[data-testid=\"result-item\"]').length > 0",
            Duration::from_secs(10),
        )
        .await;
        assert!(
            has_results,
            "Expected seeded search results for query 'rust'"
        );

        let result_count: u32 = ctx
            .page
            .evaluate("() => document.querySelectorAll('[data-testid=\"result-item\"]').length")
            .await
            .context("page evaluate failed")?
            .into_value::<u32>()
            .context("page evaluate did not return u32")?;
        assert!(result_count > 0, "Expected at least one result item");

        let first_title: String = ctx
            .page
            .evaluate(
                "() => document.querySelector('[data-testid=\"result-item\"] h3')?.innerText ?? ''",
            )
            .await
            .context("failed to read first result title")?
            .into_value::<String>()
            .context("failed to deserialize first result title")?;
        assert!(
            first_title.to_lowercase().contains(query),
            "First result title '{first_title}' should contain query '{query}'"
        );
        println!("Found {result_count} result(s); first title: {first_title}");

        teardown(ctx).await;
        Ok(())
    }

    /// 3. Search with a nonsense query — verify "No results found." is displayed.
    #[tokio::test]
    async fn search_no_results() -> anyhow::Result<()> {
        let env = get_server().await?;
        let ctx = setup().await?;

        ctx.page
            .goto(env.base_url())
            .await
            .context("failed to navigate to homepage")?;

        // Set the search query (must dispatch `input` event for Leptos `bind:value`).
        fill_search_input(&ctx.page, "zzzzzzyxwvutsrqponmlkjihgfedcba").await?;

        // Click submit.
        ctx.page
            .find_element(r#"[data-testid="search-submit"]"#)
            .await
            .context("search button not found")?
            .click()
            .await
            .context("failed to click search button")?;

        // Wait for "No results found." to appear.
        let no_results = wait_for_js_true(
            &ctx.page,
            "() => document.getElementById('results')?.innerText?.includes('No results found.')",
            Duration::from_secs(10),
        )
        .await;
        assert!(
            no_results,
            "Expected 'No results found.' after searching for nonsense"
        );

        // Also verify there are no result-item elements.
        let count: u32 = ctx
            .page
            .evaluate("() => document.querySelectorAll('[data-testid=\"result-item\"]').length")
            .await
            .context("page evaluate failed")?
            .into_value::<u32>()
            .context("page evaluate did not return u32")?;
        assert_eq!(count, 0, "Expected zero result items for nonsense query");

        teardown(ctx).await;
        Ok(())
    }

    /// 4. Live-feed page loads — navigate to `/live`, verify a heading is visible,
    ///    and a connection-status indicator is present.
    #[tokio::test]
    async fn live_feed_page_loads() -> anyhow::Result<()> {
        let env = get_server().await?;
        let ctx = setup().await?;

        let live_url = format!("{}/live", env.base_url());

        ctx.page
            .goto(&live_url)
            .await
            .context("failed to navigate to /live")?;

        // A heading should be visible (<h2>Live Feed</h2>).
        let _heading = wait_for_element(&ctx.page, "h2", Duration::from_secs(5))
            .await
            .context("no h2 heading on /live")?;
        assert!(
            element_is_visible(&ctx.page, "h2").await,
            "Heading should be visible on live feed page"
        );

        // Connection status indicator should appear (either "Connected" or "Connecting …").
        let status_indicator = wait_for_js_true(
            &ctx.page,
            "() => { const el = document.querySelector('[data-testid=\"sse-status\"]'); \
             return el && (el.innerText.includes('Connected') || el.innerText.includes('Connecting')); }",
            Duration::from_secs(10),
        )
        .await;
        assert!(
            status_indicator,
            "Expected connection status indicator on /live"
        );

        teardown(ctx).await;
        Ok(())
    }

    /// 4b. Live-feed browser-side end-to-end — navigates to `/live`, inserts a
    ///     unique sentinel row via `PostgreSQL` `INSERT`, and verifies the sentinel
    ///     title appears inside `#live-results` (proving the full
    ///     `PostgreSQL` → `PgListener` → broadcast → SSE → browser
    ///     `EventSource` → Leptos signal pipeline works end to end).
    #[tokio::test]
    async fn live_feed_receives_sse_event_in_browser() -> anyhow::Result<()> {
        let env = get_server().await?;
        let conn_str = env.db().connection_string().to_string();
        let pool = sqlx::PgPool::connect(&conn_str)
            .await
            .with_context(|| format!("failed to connect to {conn_str}"))?;

        // ── Pre-clean: wipe leftover browser-sse-sentinel rows from prior failed runs.
        sqlx::query("DELETE FROM search_results WHERE title LIKE 'browser-sse-sentinel-%'")
            .execute(&pool)
            .await
            .ok();

        let ctx = setup().await?;

        let live_url = format!("{}/live", env.base_url());
        ctx.page
            .goto(&live_url)
            .await
            .context("failed to navigate to /live")?;

        // Wait for the page to confirm the EventSource is open — guarantees the
        // browser is subscribed to the broadcast channel BEFORE we insert the
        // sentinel row. Without this, the row could land before the EventSource
        // handler attaches and the test would race the connection.
        let connected = wait_for_js_true(
            &ctx.page,
            "() => { const el = document.querySelector('[data-testid=\"sse-status\"]'); \
             return el && el.innerText.includes('Connected'); }",
            Duration::from_secs(10),
        )
        .await;
        assert!(
            connected,
            "Expected 'Connected' indicator before inserting sentinel"
        );

        // Insert a sentinel row that will trigger `notify_search_result` →
        // broadcast → SSE.
        let title = format!(
            "browser-sse-sentinel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        let sentinel_url = format!("https://example.com/{title}");
        let snippet = "Browser-side SSE end-to-end test sentinel";

        sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
            .bind(&title)
            .bind(&sentinel_url)
            .bind(snippet)
            .execute(&pool)
            .await
            .context("failed to insert sentinel row")?;

        // Wait for the sentinel title to appear in the live-results list.
        // The title text is set inside `<h3>{r.title}</h3>` in LiveFeedPage
        // (live-search/src/app.rs:345).
        let escaped_title = title.replace('\'', "\\'");
        let sentinel_appeared = wait_for_js_true(
            &ctx.page,
            &format!(
                "() => Array.from(document.querySelectorAll('#live-results h3'))\
                 .some(h => h.innerText === '{escaped_title}')"
            ),
            Duration::from_secs(10),
        )
        .await;
        assert!(
            sentinel_appeared,
            "Sentinel title '{title}' did not appear in #live-results within 10s"
        );

        // Best-effort cleanup so re-runs don't accumulate rows.
        if let Err(e) = sqlx::query("DELETE FROM search_results WHERE title = $1")
            .bind(&title)
            .execute(&pool)
            .await
        {
            eprintln!("warning: failed to delete sentinel row '{title}': {e}");
        }

        teardown(ctx).await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HTTP-level tests (no browser required)
// ---------------------------------------------------------------------------

/// 5. Server function catch-all (Pattern 9) — POST to `/api/search` with a
///     URL-encoded body and verify the response is JSON containing search
///     results.  Leptos 0.8's `#[server(endpoint = "/api/search")]` registers
///     at the doubled path `/api/api/search`; the catch-all handler in
///     `main.rs` rewrites the request so clients calling `/api/search` still
///     reach the function.  This test verifies that rewrite directly, without
///     relying on browser hydration.
///
///     Seeds its own test data so the testcontainer database is populated.
#[tokio::test]
async fn server_fn_search_returns_results_via_http() -> anyhow::Result<()> {
    let env = get_server().await?;

    // ── 1. Seed test data ────────────────────────────────────────────
    // The testcontainer DB starts empty; insert rows the server function
    // can find via FTS.
    let conn_str = env.db().connection_string().to_string();
    let pool = sqlx::PgPool::connect(&conn_str)
        .await
        .with_context(|| format!("failed to connect to {conn_str}"))?;
    let seed_rows = vec![
        (
            "Rust Programming Guide",
            "https://example.com/rust-guide",
            "Learn the Rust programming language with practical examples",
        ),
        (
            "Rust vs C++ Performance",
            "https://example.com/rust-vs-cpp",
            "A detailed comparison of Rust and C++ performance benchmarks",
        ),
        (
            "Getting Started with WebAssembly",
            "https://example.com/wasm-intro",
            "Build WebAssembly modules using Rust and wasm-pack",
        ),
        (
            "Python Data Science Cookbook",
            "https://example.com/python-ds",
            "Data science and machine learning with Python",
        ),
        (
            "TypeScript Handbook",
            "https://example.com/ts-handbook",
            "Comprehensive guide to TypeScript types and patterns",
        ),
    ];

    for (title, url, snippet) in &seed_rows {
        sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
            .bind(title)
            .bind(url)
            .bind(snippet)
            .execute(&pool)
            .await
            .context("failed to insert seed data")?;
    }

    // ── 2. Call the server function ──────────────────────────────────
    let url = format!("{}/api/search", env.base_url());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;

    // Leptos 0.8 server fns use URL encoding by default (not JSON).
    let response = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("query=rust")
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;

    assert_eq!(
        response.status(),
        200,
        "Expected HTTP 200 from /api/search, got {}",
        response.status()
    );

    let json: serde_json::Value = response
        .json()
        .await
        .context("server fn response is not valid JSON")?;

    // The response is a JSON array of SearchResult objects.
    let results = json
        .as_array()
        .context("server fn response should be a JSON array")?;

    assert!(
        !results.is_empty(),
        "Expected at least one search result for query 'rust'"
    );

    // FTS ranking is not deterministic across postgres versions / configs, so
    // we don't assume a specific row comes first. Instead, assert that at
    // least one of the returned rows contains "rust" (case-insensitive) in
    // either its title or its snippet. This guarantees the search hit
    // matching works without coupling to a particular ranking order.
    let any_rust_hit = results.iter().any(|r| {
        let title = r
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let snippet = r
            .get("snippet")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        title.to_lowercase().contains("rust") || snippet.to_lowercase().contains("rust")
    });
    anyhow::ensure!(
        any_rust_hit,
        "Expected at least one search result for query 'rust' (title or snippet containing 'rust'), got: {json}"
    );

    // Every returned row must have the documented schema fields.
    for (idx, row) in results.iter().enumerate() {
        anyhow::ensure!(
            row.get("title")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "row[{idx}] missing string 'title' field: {row}"
        );
        anyhow::ensure!(
            row.get("url").is_some(),
            "row[{idx}] missing 'url' field: {row}"
        );
    }

    println!(
        "Server fn returned {} result(s); first title: {}",
        results.len(),
        results[0]
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?")
    );
    Ok(())
}

/// 6. Root path is reachable — GET `/` returns either SSR HTML or the
///    in-process fallback (depending on whether Leptos build artifacts
///    are present at `live-search/pkg/`).
#[tokio::test]
async fn root_path_reachable() -> anyhow::Result<()> {
    let env = get_server().await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;
    let resp = client
        .get(env.base_url())
        .send()
        .await
        .with_context(|| format!("failed to GET {}", env.base_url()))?;
    anyhow::ensure!(
        resp.status().is_success() || resp.status().is_redirection(),
        "expected success or redirect from root path, got {}",
        resp.status()
    );
    Ok(())
}

/// 7. Static assets are served (Critical Rule 7) — GET `/pkg/live_search.js`
///    and verify HTTP 200.  Without this, SSR pages render but WASM hydration
///    never runs because the browser 404s on the JS module.
///
///    NOTE: The in-process test server only mounts `/pkg/` when a `cargo leptos build`
///    artifact is present at `../live-search/pkg/`.  When that build is absent
///    (CI without `cargo leptos`, or fresh clones) the test skips with a warning
///    — that is an environment gap, not a code regression.
#[tokio::test]
async fn static_assets_are_served() -> anyhow::Result<()> {
    let pkg_path = Path::new("../live-search/pkg/live_search.js");
    if !pkg_path.exists() {
        eprintln!(
            "WARNING: Leptos build artifacts not found at `{}`. \
             This is expected in CI without `cargo leptos build`. \
             Run `cargo leptos build` first for this test to actually verify \
             hydration assets are served at `/pkg/live_search.js`.",
            pkg_path.display()
        );
        return Ok(());
    }

    let env = get_server().await?;
    let url = format!("{}/pkg/live_search.js", env.base_url());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    let status = response.status();
    anyhow::ensure!(
        status == 200,
        "Expected HTTP 200 from /pkg/live_search.js, got {status}. \
         (Build artifacts exist at {})",
        pkg_path.display()
    );

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Verify it's actually JavaScript, not an error page.
    anyhow::ensure!(
        content_type.contains("javascript") || content_type.contains("text/plain"),
        "Expected JavaScript content-type, got '{content_type}'"
    );
    Ok(())
}

/// Manual verification: run `cargo leptos build`, then run this test to
/// confirm WASM/JS assets are served at `/pkg/`.
///
/// Unlike [`static_assets_are_served`], this test **asserts** 200 and will
/// fail if the Leptos build artifacts are missing.
#[tokio::test]
#[ignore = "run after `cargo leptos build` to verify hydration assets"]
async fn static_assets_are_served_when_built() -> anyhow::Result<()> {
    let env = get_server().await?;
    let url = format!("{}/pkg/live_search.js", env.base_url());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;
    anyhow::ensure!(
        response.status() == 200,
        "Expected HTTP 200 from /pkg/live_search.js (need `cargo leptos build`), got {}",
        response.status()
    );
    Ok(())
}

/// 8. Unknown path returns 404 — verifies the fallback handler in
///    `live-search/src/main.rs` returns 404 for unmatched routes.
#[tokio::test]
async fn unknown_path_returns_404() -> anyhow::Result<()> {
    let env = get_server().await?;

    let url = format!("{}/nonexistent-path-{}", env.base_url(), std::process::id());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    assert_eq!(
        response.status(),
        404,
        "Expected HTTP 404 for unknown path, got {}",
        response.status()
    );
    Ok(())
}
