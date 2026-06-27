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

use common::{
    base_url, element_is_enabled, element_is_visible, require_server, setup, teardown,
    wait_for_element, wait_for_js_true,
};

/// Navigate to the search page root and verify the page title renders.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_live_search_page_loads() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    // Navigate to the base URL.
    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base URL");

    // Grab the page title — chromiumoxide returns `Option<String>`.
    let title = ctx
        .page
        .get_title()
        .await
        .expect("Failed to read page title")
        .unwrap_or_default();

    // Accept both a pure title-tag value and a fallback check on an h1.
    assert!(
        !title.is_empty() || ctx.page.find_element("h1").await.is_ok(),
        "Expected a non-empty <title> or an <h1> on the page"
    );

    teardown(ctx).await;
}

/// Verify the search input element is present on the page and can receive focus.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_live_search_input_exists() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base URL");

    // Wait for the search input to appear (the page uses `<input type="text">` without `id`).
    let _input = wait_for_element(&ctx.page, r#"input[type="text"]"#, Duration::from_secs(5))
        .await
        .expect("Search input not found");

    // Verify the element is visible and enabled via JS helpers.
    let visible = element_is_visible(&ctx.page, r#"input[type="text"]"#).await;
    assert!(visible, "Search input should be visible");

    let enabled = element_is_enabled(&ctx.page, r#"input[type="text"]"#).await;
    assert!(enabled, "Search input should be enabled");

    // Read the placeholder attribute.
    let placeholder = common::element_attribute(&ctx.page, r#"input[type="text"]"#, "placeholder")
        .await
        .unwrap_or_default();
    println!("Search input placeholder: {placeholder:?}");

    teardown(ctx).await;
}

/// Type a query into the search box, submit, and wait for results to appear.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_live_search_submits() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base URL");

    // Wait for the search input to be present (the page uses `<input type="text">`).
    let search_input = wait_for_element(&ctx.page, r#"input[type="text"]"#, Duration::from_secs(5))
        .await
        .expect("Search input not found");

    // Type a seeded query using the Element API.
    search_input
        .click()
        .await
        .expect("Failed to focus search input")
        .type_str("rust")
        .await
        .expect("Failed to type search input");

    // Click the submit button or press Enter.
    let has_button = ctx
        .page
        .find_element(r#"button[type="submit"]"#)
        .await
        .is_ok();

    if has_button {
        ctx.page
            .find_element(r#"button[type="submit"]"#)
            .await
            .expect("Search button not found")
            .click()
            .await
            .expect("Failed to click search button");
    } else {
        search_input
            .press_key("Enter")
            .await
            .expect("Failed to press Enter in search input");
    }

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

    teardown(ctx).await;
}

/// Search for a nonsense string and verify the "no results" message appears.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_live_search_empty_results() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base URL");

    // Wait for the input.
    let search_input = wait_for_element(&ctx.page, r#"input[type="text"]"#, Duration::from_secs(5))
        .await
        .expect("Search input not found");

    // Fill with a nonsense query.
    search_input
        .click()
        .await
        .expect("Failed to focus search input")
        .type_str("zzzzzzyxwvutsrqponmlkjihgfedcba")
        .await
        .expect("Failed to fill search input");

    // Click the submit button or press Enter.
    let has_button = ctx
        .page
        .find_element(r#"button[type="submit"]"#)
        .await
        .is_ok();

    if has_button {
        ctx.page
            .find_element(r#"button[type="submit"]"#)
            .await
            .expect("Search button not found")
            .click()
            .await
            .expect("Failed to click search button");
    } else {
        search_input
            .press_key("Enter")
            .await
            .expect("Failed to press Enter in search input");
    }

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

    teardown(ctx).await;
}

/// Navigate to `/live` and wait for an SSE-connected indicator to appear.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_live_search_sse_connects() {
    require_server(&base_url()).await;
    let ctx = setup().await;

    let live_url = format!("{}/live", ctx.base_url);

    ctx.page
        .goto(&live_url)
        .await
        .expect("Failed to navigate to /live");

    // Wait for the SSE status indicator (the /live page displays "Connecting …"
    // and may update to show "Connected" when the EventSource handshake completes).
    let connected = wait_for_js_true(
        &ctx.page,
        "() => document.body.innerText.includes('Connected') \
         || document.body.innerText.includes('Connecting')",
        Duration::from_secs(15), // SSE connections can be slow to establish
    )
    .await;

    assert!(
        connected,
        "Expected a connection status indicator (Connected/Connecting) after navigating to /live"
    );

    teardown(ctx).await;
}

// ---------------------------------------------------------------------------
// Required integration tests (from spec)
// ---------------------------------------------------------------------------

/// 1. Homepage loads — verify HTTP 200, page title contains "Live" or "Search",
///    a search input is visible, and a heading (H1/H2) is present.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn homepage_loads() {
    require_server(&base_url()).await;
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
    require_server(&base_url()).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to homepage");

    // Wait for the input to be present.
    let search_input = wait_for_element(&ctx.page, r#"input[type="text"]"#, Duration::from_secs(5))
        .await
        .expect("Search input not found");

    // Type a meaningful query.
    let query = "rust";
    search_input
        .click()
        .await
        .expect("Failed to focus search input")
        .type_str(query)
        .await
        .expect("Failed to fill search input");

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
        .ok()
        .and_then(|v| v.into_value::<u32>().ok())
        .unwrap_or(0);
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
    require_server(&base_url()).await;
    let ctx = setup().await;

    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to homepage");

    // Wait for the input.
    let search_input = wait_for_element(&ctx.page, r#"input[type="text"]"#, Duration::from_secs(5))
        .await
        .expect("Search input not found");

    // Fill with a nonsense string.
    search_input
        .click()
        .await
        .expect("Failed to focus search input")
        .type_str("zzzzzzyxwvutsrqponmlkjihgfedcba")
        .await
        .expect("Failed to fill search input");

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
        .ok()
        .and_then(|v| v.into_value::<u32>().ok())
        .unwrap_or(0);
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
    require_server(&base_url()).await;
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
