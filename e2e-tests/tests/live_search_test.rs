//! E2E tests for a live-search example.
//!
//! These tests verify:
//! - The search page loads and renders its title.
//! - A search input is present on the page.
//! - Typing a query and submitting shows results.
//! - Searching for nonsense yields a "no results" message.
//! - An SSE live-feed indicator appears on the `/live` route.
//! - The server function catch-all (Pattern 9) routes `/api/search` correctly.
//! - Static WASM/JS assets are served from `/pkg/` (Critical Rule 7).
//! - Unknown paths return 404 via the fallback handler.
//!
//! All tests are gated behind the `integration` feature and will be ignored
//! when running plain `cargo test`.  Use `--features integration` to enable
//! them, and make sure the live-search service is running on port 3000.

use std::time::Duration;

mod common;

use chromiumoxide::Page;
use common::{
    element_is_visible, require_server, setup, teardown, wait_for_element, wait_for_js_true,
};
use e2e_tests::base_url;

// ---------------------------------------------------------------------------
// Required integration tests (from spec)
// ---------------------------------------------------------------------------

/// Fill the search input by setting its value and dispatching a single
/// `input` event. Leptos 0.8's `bind:value` listens for the `input` event
/// to update the underlying signal; `WebElement::type_str` types characters
/// one at a time without firing the right input event on every Keystroke,
/// so we set the value directly and dispatch a single bubbleable event.
async fn fill_search_input(page: &Page, query: &str) {
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
    let value_json = serde_json::to_string(query).expect("query is always valid JSON");
    let script = format!(
        r#"(() => {{
            const el = document.querySelector('input[type="text"]');
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
        .expect("Failed to set search input value and dispatch input event")
        .into_value::<String>()
        .expect("Failed to deserialize echoed value");
    assert_eq!(
        echoed, query,
        "value was not set correctly: browser reports '{echoed}', wanted '{query}'"
    );
}

/// 1. Homepage loads — verify HTTP 200, page title contains "Live" or "Search",
///    a search input is visible, and a heading (H1/H2) is present.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn homepage_loads() {
    require_server(&base_url(None)).await;
    let ctx = setup().await;

    // Navigate — goto resolves after the page is fully loaded.
    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to homepage");

    // Page title is the literal value set by `<Title text="Live Search" />`
    // in live-search/src/app.rs. Tightening the assertion to the exact value
    // catches typos in the Title component that a substring check would miss.
    let title = ctx
        .page
        .get_title()
        .await
        .expect("Failed to read page title")
        .unwrap_or_default();
    assert_eq!(
        title, "Live Search",
        "Page title should be exactly 'Live Search', got '{title}'"
    );

    // Search input is visible.
    let _input = wait_for_element(&ctx.page, r#"input[type="text"]"#, Duration::from_secs(5))
        .await
        .expect("No search input found on the page");
    assert!(
        element_is_visible(&ctx.page, r#"input[type="text"]"#).await,
        "Search input should be visible"
    );

    // A heading (H2) is visible.
    let _heading = wait_for_element(&ctx.page, "h2", Duration::from_secs(5))
        .await
        .expect("No h2 heading found");
    assert!(
        element_is_visible(&ctx.page, "h2").await,
        "H2 heading should be visible"
    );

    teardown(ctx).await;
}

/// 2. Search returns results — type a query, submit, wait for result items.
///    Asserts at least one `.result-item` appears and its text contains the
///    query substring.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn search_returns_results() {
    require_server(&base_url(None)).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to homepage");

    // Wait for the input to be present.
    let _search_input =
        wait_for_element(&ctx.page, r#"input[type="text"]"#, Duration::from_secs(5))
            .await
            .expect("Search input not found");

    // Set the search query (must dispatch `input` event for Leptos `bind:value`).
    let query = "rust";
    fill_search_input(&ctx.page, query).await;

    // Click the submit button.
    ctx.page
        .find_element(r#"button[type="submit"]"#)
        .await
        .expect("Search button not found")
        .click()
        .await
        .expect("Failed to click search button");

    let has_results = wait_for_js_true(
        &ctx.page,
        "() => document.querySelectorAll('.result-item').length > 0",
        Duration::from_secs(10),
    )
    .await;
    assert!(
        has_results,
        "Expected seeded search results for query 'rust'"
    );

    let result_count: u32 = ctx
        .page
        .evaluate("() => document.querySelectorAll('.result-item').length")
        .await
        .expect("evaluate failed")
        .into_value::<u32>()
        .expect("not a u32");
    assert!(result_count > 0, "Expected at least one result item");

    let first_title: String = ctx
        .page
        .evaluate("() => document.querySelector('.result-item h3')?.innerText ?? ''")
        .await
        .expect("Failed to read first result title")
        .into_value::<String>()
        .expect("Failed to deserialize first result title");
    assert!(
        first_title.to_lowercase().contains(query),
        "First result title '{first_title}' should contain query '{query}'"
    );
    println!("Found {result_count} result(s); first title: {first_title}");

    teardown(ctx).await;
}

/// 3. Search with a nonsense query — verify "No results found." is displayed.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn search_no_results() {
    require_server(&base_url(None)).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to homepage");

    // Set the search query (must dispatch `input` event for Leptos `bind:value`).
    fill_search_input(&ctx.page, "zzzzzzyxwvutsrqponmlkjihgfedcba").await;

    // Click submit.
    ctx.page
        .find_element(r#"button[type="submit"]"#)
        .await
        .expect("Search button not found")
        .click()
        .await
        .expect("Failed to click search button");

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

    // Also verify there are no .result-item elements.
    let count: u32 = ctx
        .page
        .evaluate("() => document.querySelectorAll('.result-item').length")
        .await
        .expect("evaluate failed")
        .into_value::<u32>()
        .expect("not a u32");
    assert_eq!(count, 0, "Expected zero result items for nonsense query");

    teardown(ctx).await;
}

/// 4. Live-feed page loads — navigate to `/live`, verify a heading is visible,
///    and a connection-status indicator is present.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn live_feed_page_loads() {
    require_server(&base_url(None)).await;
    let ctx = setup().await;

    let live_url = format!("{}/live", ctx.base_url);

    ctx.page
        .goto(&live_url)
        .await
        .expect("Failed to navigate to /live");

    // A heading should be visible (<h2>Live Feed</h2>).
    let _heading = wait_for_element(&ctx.page, "h2", Duration::from_secs(5))
        .await
        .expect("No h2 heading on /live");
    assert!(
        element_is_visible(&ctx.page, "h2").await,
        "Heading should be visible on live feed page"
    );

    // Connection status indicator should appear (either "Connected" or "Connecting …").
    let status_indicator = wait_for_js_true(
        &ctx.page,
        "() => document.body.innerText.includes('Connected') \
         || document.body.innerText.includes('Connecting')",
        Duration::from_secs(10),
    )
    .await;
    assert!(
        status_indicator,
        "Expected connection status indicator on /live"
    );

    teardown(ctx).await;
}

/// 4b. Live-feed browser-side end-to-end — navigates to `/live`, inserts a
///     unique sentinel row via `PostgreSQL` `INSERT`, and verifies the sentinel
///     title appears inside `#live-results` (proving the full
///     `PostgreSQL` → `PgListener` → broadcast → SSE → browser
///     `EventSource` → Leptos signal pipeline works end to end).
///
///     This test covers the gap that the `live_feed_page_loads` test above
///     leaves: that test only verifies the static "Connecting/Connected"
///     indicator, which appears the moment the `EventSource` opens, regardless
///     of whether any data event is ever delivered. The HTTP-level
///     `sse_test::notify_trigger_fires_sse_event` covers the server side, but
///     no test verified that the BROWSER actually receives a live event.
///
///     Requires `DATABASE_URL` to be set in the environment in addition to
///     `--features integration`.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration and DATABASE_URL"
)]
async fn live_feed_receives_sse_event_in_browser() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");

    require_server(&base_url(None)).await;

    // ── Pre-clean: wipe leftover browser-sse-sentinel rows from prior failed runs.
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to database");
    sqlx::query("DELETE FROM search_results WHERE title LIKE 'browser-sse-sentinel-%'")
        .execute(&pool)
        .await
        .ok();

    let ctx = setup().await;

    let live_url = format!("{}/live", ctx.base_url);
    ctx.page
        .goto(&live_url)
        .await
        .expect("Failed to navigate to /live");

    // Wait for the page to confirm the EventSource is open — guarantees the
    // browser is subscribed to the broadcast channel BEFORE we insert the
    // sentinel row. Without this, the row could land before the EventSource
    // handler attaches and the test would race the connection.
    let connected = wait_for_js_true(
        &ctx.page,
        "() => document.body.innerText.includes('Connected')",
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
        .expect("Failed to insert sentinel row");

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
    pool.close().await;

    teardown(ctx).await;
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
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn server_fn_search_returns_results_via_http() {
    require_server(&base_url(None)).await;

    let url = format!("{}/api/search", base_url(None));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build reqwest client");

    // Leptos 0.8 server fns use URL encoding by default (not JSON).
    let response = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("query=rust")
        .send()
        .await
        .unwrap_or_else(|e| panic!("Failed to POST {url}: {e}"));

    assert_eq!(
        response.status(),
        200,
        "Expected HTTP 200 from /api/search, got {}",
        response.status()
    );

    let json: serde_json::Value = response
        .json()
        .await
        .expect("Server fn response is not valid JSON");

    // The response is a JSON array of SearchResult objects.
    let results = json
        .as_array()
        .expect("Server fn response should be a JSON array");

    assert!(
        !results.is_empty(),
        "Expected at least one search result for query 'rust'"
    );

    // Verify the first result has the expected fields.
    let first = &results[0];
    assert!(
        first.get("title").is_some(),
        "Search result should have a 'title' field"
    );
    assert!(
        first.get("url").is_some(),
        "Search result should have a 'url' field"
    );

    println!(
        "Server fn returned {} result(s); first title: {}",
        results.len(),
        first.get("title").and_then(|v| v.as_str()).unwrap_or("?")
    );
}

/// 6. Static assets are served (Critical Rule 7) — GET `/pkg/live_search.js`
///    and verify HTTP 200.  Without this, SSR pages render but WASM hydration
///    never runs because the browser 404s on the JS module.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn static_assets_are_served() {
    require_server(&base_url(None)).await;

    let url = format!("{}/pkg/live_search.js", base_url(None));
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
        "Expected HTTP 200 from /pkg/live_search.js, got {} — hydration will fail without this",
        response.status()
    );

    // Verify it's actually JavaScript, not an error page.
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("javascript") || content_type.contains("text/plain"),
        "Expected JavaScript content-type, got '{content_type}'"
    );
}

/// 7. Unknown path returns 404 — verifies the fallback handler in
///    `live-search/src/main.rs` returns 404 for unmatched routes.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn unknown_path_returns_404() {
    require_server(&base_url(None)).await;

    let url = format!("{}/nonexistent-path-{}", base_url(None), std::process::id());
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
        404,
        "Expected HTTP 404 for unknown path, got {}",
        response.status()
    );
}
