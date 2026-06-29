//! E2E tests for a live-search example.
//!
//! These tests verify:
//! - The search page loads and renders its title.
//! - A search input is present on the page.
//! - Typing a query and submitting shows results.
//! - Searching for nonsense yields a "no results" message.
//! - An SSE live-feed indicator appears on the `/live` route.
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

    // Page title contains "Live" or "Search"
    let title = ctx
        .page
        .get_title()
        .await
        .expect("Failed to read page title")
        .unwrap_or_default();
    assert!(
        title.contains("Live") || title.contains("Search"),
        "Page title '{title}' should contain 'Live' or 'Search'"
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
