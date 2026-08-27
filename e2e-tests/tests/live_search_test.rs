//! End-to-end tests for the live-search example.
//!
//! HTTP tests exercise the same Leptos route tree as production against an
//! isolated Postgres testcontainer. Browser tests additionally exercise WASM
//! hydration and the full PostgreSQL -> LISTEN/NOTIFY -> SSE -> browser path.

use std::time::Duration;

use anyhow::Context;

#[cfg(feature = "browser-tests")]
use chromiumoxide::Page;
#[cfg(feature = "browser-tests")]
use e2e_tests::common::{element_is_visible, setup, teardown, wait_for_element, wait_for_js_true};
use e2e_tests::common::{LiveSearchEnv, SharedServer};

static SERVER: SharedServer<LiveSearchEnv> = SharedServer::new();

async fn get_server() -> anyhow::Result<&'static LiveSearchEnv> {
    SERVER.get(|| async { LiveSearchEnv::start().await }).await
}

#[cfg(feature = "browser-tests")]
mod browser_tests {
    use super::*;

    async fn fill_search_input(page: &Page, query: &str) -> anyhow::Result<()> {
        tokio::time::sleep(Duration::from_millis(750)).await;

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

    #[tokio::test]
    async fn search_returns_results() -> anyhow::Result<()> {
        let env = get_server().await?;
        let ctx = setup().await?;

        ctx.page
            .goto(env.base_url())
            .await
            .context("failed to navigate to homepage")?;

        let _search_input = wait_for_element(
            &ctx.page,
            r#"[data-testid="search-input"]"#,
            Duration::from_secs(5),
        )
        .await
        .context("search input not found")?;

        let query = "rust";
        fill_search_input(&ctx.page, query).await?;
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
        assert!(has_results, "Expected a seeded result for query 'rust'");

        let result_count: u32 = ctx
            .page
            .evaluate("() => document.querySelectorAll('[data-testid=\"result-item\"]').length")
            .await
            .context("page evaluate failed")?
            .into_value::<u32>()
            .context("page evaluate did not return u32")?;
        assert!(result_count > 0, "Expected at least one result item");

        // leptos-struct-table renders the title as an <a> inside the first cell,
        // not an <h3>. Keep the selector coupled to the actual production DOM.
        let first_title: String = ctx
            .page
            .evaluate(
                "() => document.querySelector('[data-testid=\"result-item\"] a')?.innerText ?? ''",
            )
            .await
            .context("failed to read first result title")?
            .into_value::<String>()
            .context("failed to deserialize first result title")?;
        assert!(
            first_title.to_lowercase().contains(query),
            "First result title '{first_title}' should contain query '{query}'"
        );

        teardown(ctx).await;
        Ok(())
    }

    #[tokio::test]
    async fn search_no_results() -> anyhow::Result<()> {
        let env = get_server().await?;
        let ctx = setup().await?;

        ctx.page
            .goto(env.base_url())
            .await
            .context("failed to navigate to homepage")?;
        fill_search_input(&ctx.page, "zzzzzzyxwvutsrqponmlkjihgfedcba").await?;
        ctx.page
            .find_element(r#"[data-testid="search-submit"]"#)
            .await
            .context("search button not found")?
            .click()
            .await
            .context("failed to click search button")?;

        let no_results = wait_for_js_true(
            &ctx.page,
            "() => document.getElementById('results')?.innerText?.includes('No results found.')",
            Duration::from_secs(10),
        )
        .await;
        assert!(no_results, "Expected 'No results found.' for nonsense query");

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

    #[tokio::test]
    async fn live_feed_page_loads() -> anyhow::Result<()> {
        let env = get_server().await?;
        let ctx = setup().await?;
        let live_url = format!("{}/live", env.base_url());

        ctx.page
            .goto(&live_url)
            .await
            .context("failed to navigate to /live")?;
        let _heading = wait_for_element(&ctx.page, "h2", Duration::from_secs(5))
            .await
            .context("no h2 heading on /live")?;
        assert!(element_is_visible(&ctx.page, "h2").await);

        let status_indicator = wait_for_js_true(
            &ctx.page,
            "() => { const el = document.querySelector('[data-testid=\"sse-status\"]'); \
             return el && (el.innerText.includes('Connected') || el.innerText.includes('Connecting')); }",
            Duration::from_secs(10),
        )
        .await;
        assert!(status_indicator, "Expected SSE connection status indicator");

        teardown(ctx).await;
        Ok(())
    }

    #[tokio::test]
    async fn live_feed_receives_sse_event_in_browser() -> anyhow::Result<()> {
        let env = get_server().await?;
        let conn_str = env.db().connection_string().to_string();
        let pool = sqlx::PgPool::connect(&conn_str)
            .await
            .with_context(|| format!("failed to connect to {conn_str}"))?;

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

        // Connected is now set only after the server's named `connected` event
        // is actually received, so this is a real subscription barrier.
        let connected = wait_for_js_true(
            &ctx.page,
            "() => document.querySelector('[data-testid=\"sse-status\"]')?.innerText.includes('Connected')",
            Duration::from_secs(10),
        )
        .await;
        assert!(connected, "Expected a real SSE connected event");

        let title = format!(
            "browser-sse-sentinel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        let sentinel_url = format!("https://example.com/{title}");
        sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
            .bind(&title)
            .bind(&sentinel_url)
            .bind("Browser-side SSE end-to-end test sentinel")
            .execute(&pool)
            .await
            .context("failed to insert sentinel row")?;

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
        assert!(sentinel_appeared, "Sentinel did not reach the browser via SSE");

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

#[tokio::test]
async fn server_fn_search_returns_results_via_http() -> anyhow::Result<()> {
    let env = get_server().await?;
    let conn_str = env.db().connection_string().to_string();
    let pool = sqlx::PgPool::connect(&conn_str)
        .await
        .with_context(|| format!("failed to connect to {conn_str}"))?;

    let seed_rows = [
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
    ];
    for (title, url, snippet) in seed_rows {
        sqlx::query(
            "INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3) \
             ON CONFLICT (url) DO UPDATE SET title = EXCLUDED.title, snippet = EXCLUDED.snippet",
        )
        .bind(title)
        .bind(url)
        .bind(snippet)
        .execute(&pool)
        .await
        .context("failed to insert seed data")?;
    }

    // `endpoint = "search"` is relative to Leptos's default `/api` prefix.
    let url = format!("{}/api/search", env.base_url());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;
    let response = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("query=rust")
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response
        .json()
        .await
        .context("server fn response is not valid JSON")?;
    let results = json
        .as_array()
        .context("server fn response should be a JSON array")?;
    assert!(!results.is_empty());
    assert!(results.iter().any(|row| {
        row.get("title")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|title| title.to_lowercase().contains("rust"))
    }));
    Ok(())
}

#[tokio::test]
async fn root_path_is_real_ssr_page() -> anyhow::Result<()> {
    let env = get_server().await?;
    let response = reqwest::get(env.base_url()).await?;
    anyhow::ensure!(response.status().is_success());
    let body = response.text().await?;
    anyhow::ensure!(
        body.contains("data-testid=\"search-input\"") || body.contains("Enter search query"),
        "root did not render the production search page"
    );
    Ok(())
}

#[cfg(feature = "browser-tests")]
#[tokio::test]
async fn static_assets_are_served() -> anyhow::Result<()> {
    let pkg_dir = std::env::var_os("LIVE_SEARCH_PKG_DIR")
        .map(std::path::PathBuf::from)
        .context("LIVE_SEARCH_PKG_DIR is required for browser tests")?;
    let pkg_path = pkg_dir.join("live_search.js");
    anyhow::ensure!(pkg_path.exists(), "missing {}", pkg_path.display());

    let env = get_server().await?;
    let url = format!("{}/pkg/live_search.js", env.base_url());
    let response = reqwest::get(&url).await?;
    anyhow::ensure!(response.status() == 200);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    anyhow::ensure!(
        content_type.contains("javascript") || content_type.contains("text/plain")
    );
    let bytes = response.bytes().await?;
    anyhow::ensure!(bytes.len() > 1024, "hydration JS is suspiciously small");
    Ok(())
}

#[cfg(feature = "browser-tests")]
#[tokio::test]
async fn stylance_css_is_generated() -> anyhow::Result<()> {
    let pkg_dir = std::env::var_os("LIVE_SEARCH_PKG_DIR")
        .map(std::path::PathBuf::from)
        .context("LIVE_SEARCH_PKG_DIR is required for browser tests")?;
    let css_path = pkg_dir.join("live-search.css");
    anyhow::ensure!(css_path.exists(), "missing {}", css_path.display());

    let env = get_server().await?;
    let response = reqwest::get(format!("{}/pkg/live-search.css", env.base_url())).await?;
    anyhow::ensure!(response.status() == 200);
    let body = response.text().await?;
    anyhow::ensure!(body.len() > 100, "Stylance CSS is suspiciously small");

    // `str::contains` is not a regex. Check the emitted `<name>-<hex>` suffix
    // structurally instead of looking for the literal text "[0-9a-fA-F]".
    let has_hashed_class = body.split('-').any(|tail| {
        let prefix: String = tail.chars().take(6).collect();
        prefix.len() == 6 && prefix.chars().all(|ch| ch.is_ascii_hexdigit())
    });
    anyhow::ensure!(has_hashed_class, "no Stylance hashed class selector found");
    Ok(())
}

#[tokio::test]
async fn unknown_path_returns_404() -> anyhow::Result<()> {
    let env = get_server().await?;
    let url = format!("{}/nonexistent-path-{}", env.base_url(), std::process::id());
    let response = reqwest::get(url).await?;
    assert_eq!(response.status(), 404);
    Ok(())
}
