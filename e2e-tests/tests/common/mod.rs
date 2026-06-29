//! Integration-test-only helpers — browser setup, teardown, element waits.
//!
//! Each integration test creates an isolated [`TestContext`] via [`setup`],
//! runs assertions, then cleans up via [`teardown`]. Browsers run headless.
//! The `base_url` points to the example server under test (defaults to
//! `http://localhost:3000`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::Page;
use futures::StreamExt;

use e2e_tests::base_url;

/// Holds the browser, page, base URL, and profile directory for a single test.
///
/// Fields may appear unused in some test binaries (each `tests/*.rs` file
/// compiles its own copy of this module); suppress the warning.
#[allow(dead_code)]
#[derive(Debug)]
pub struct TestContext {
    pub browser: Browser,
    pub page: Page,
    pub base_url: String,
    pub profile_dir: PathBuf,
}

/// Generate a unique Chromium user-data-dir path using PID, nanos-since-epoch,
/// and an atomic counter.  The counter ensures uniqueness even when two threads
/// call this at the same monotonic instant.
#[allow(dead_code)]
fn unique_profile_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_nanos();
    let dir = format!(
        "/tmp/chromiumoxide-{pid}-{nanos}-{n}",
        pid = std::process::id(),
    );
    PathBuf::from(dir)
}

/// Poll a server until it responds with an HTTP 2xx / 3xx status, or the timeout
/// elapses.  Returns `true` if the server became healthy, `false` if the timeout
/// was reached.
pub async fn wait_for_server(url: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(resp) = reqwest::get(url).await
            && (resp.status().is_success() || resp.status().is_redirection())
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Initialise chromiumoxide, launch a headless Chromium browser, create a page,
/// and return a [`TestContext`].
///
/// # Panics
/// Panics if the browser cannot launch or the page cannot be created.
#[allow(dead_code)]
pub async fn setup() -> TestContext {
    let chrome_path = std::env::var("CHROME_PATH").ok().or_else(|| {
        let playwright_path = format!(
            "{}/chromium-1208/chrome-linux64/chrome",
            std::env::var("PLAYWRIGHT_BROWSERS_PATH")
                .unwrap_or_else(|_| "/home/jan/.cache/ms-playwright".to_string())
        );
        std::path::Path::new(&playwright_path)
            .exists()
            .then_some(playwright_path)
    });

    let profile_dir = unique_profile_dir();
    std::fs::create_dir_all(&profile_dir).expect("failed to create Chromium profile dir");

    let mut builder = BrowserConfig::builder()
        .user_data_dir(profile_dir.clone())
        .no_sandbox();
    if let Some(chrome_path) = chrome_path {
        builder = builder.chrome_executable(chrome_path);
    }
    let config = builder.build().expect("Failed to build BrowserConfig");

    let (browser, mut handler) = Browser::launch(config)
        .await
        .expect("Failed to launch Chromium browser");

    // Spawn the handler task — REQUIRED, otherwise the browser hangs.
    tokio::spawn(async move { while handler.next().await.is_some() {} });

    // Create a page with about:blank then navigate in each test.
    let page = browser
        .new_page("about:blank")
        .await
        .expect("Failed to create new page");

    TestContext {
        browser,
        page,
        base_url: base_url(None),
        profile_dir,
    }
}

/// Tear down a [`TestContext`] by closing the page and browser in reverse
/// creation order. Cleanup errors are printed for diagnostics but do not mask
/// the test assertion that already ran.
#[allow(dead_code)]
pub async fn teardown(ctx: TestContext) {
    let TestContext {
        mut browser,
        page,
        profile_dir,
        ..
    } = ctx;
    if let Err(e) = page.close().await {
        eprintln!("Failed to close Chromium page during teardown: {e}");
    }
    if let Err(e) = browser.close().await {
        eprintln!("Failed to close Chromium browser during teardown: {e}");
    }
    // Remove the temporary profile directory.
    if let Err(e) = std::fs::remove_dir_all(&profile_dir) {
        eprintln!(
            "Failed to remove profile dir {}: {e}",
            profile_dir.display()
        );
    }
}

/// Require the server at `url` to respond within 5 seconds.
///
/// # Panics
/// Panics if the server does not respond with a 2xx or 3xx status before the
/// timeout expires.
#[allow(dead_code)]
pub async fn require_server(url: &str) {
    assert!(
        wait_for_server(url, Duration::from_secs(5)).await,
        "server at {url} is not reachable"
    );
}

// ──────  Element wait helpers  ──────

/// Poll `page.evaluate(expression)` until it returns `true` (as a boolean)
/// or the timeout elapses. Replacement for browser-framework wait helpers.
#[allow(dead_code)]
pub async fn wait_for_js_true(page: &Page, expression: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(val) = page.evaluate(expression).await
            && val.into_value::<bool>().unwrap_or(false)
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Poll `page.find_element(selector)` until it returns `Ok` or the timeout
/// elapses.  Returns the element if found.
#[allow(dead_code)]
pub async fn wait_for_element(
    page: &Page,
    selector: &str,
    timeout: Duration,
) -> Option<chromiumoxide::element::Element> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(el) = page.find_element(selector).await {
            return Some(el);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

/// Poll `page.find_element(selector)` until the element is both present AND
/// visible (per [`element_is_visible`]), or the timeout elapses.  Returns the
/// element if found and visible.
#[allow(dead_code)]
pub async fn wait_for_visible_element(
    page: &Page,
    selector: &str,
    timeout: Duration,
) -> Option<chromiumoxide::element::Element> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(el) = page.find_element(selector).await
            && element_is_visible(page, selector).await
        {
            return Some(el);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

/// Check whether an element matching `selector` is visible on the page
/// (exists, has non-zero dimensions, `visibility` not hidden, `display` not none).
#[allow(dead_code)]
pub async fn element_is_visible(page: &Page, selector: &str) -> bool {
    let escaped_sel = selector.replace('\\', "\\\\").replace('\'', "\\'");
    let js = format!(
        "(() => {{ const el = document.querySelector('{escaped_sel}'); \
         return !!el && el.offsetWidth > 0 && el.offsetHeight > 0 && \
         getComputedStyle(el).visibility !== 'hidden' && \
         getComputedStyle(el).display !== 'none'; }})()"
    );
    page.evaluate(js)
        .await
        .ok()
        .and_then(|v| v.into_value::<bool>().ok())
        .unwrap_or(false)
}

/// Check whether an element matching `selector` is enabled (not disabled).
#[allow(dead_code)]
pub async fn element_is_enabled(page: &Page, selector: &str) -> bool {
    let escaped_sel = selector.replace('\\', "\\\\").replace('\'', "\\'");
    let js = format!(
        "(() => {{ const el = document.querySelector('{escaped_sel}'); \
         return !!el && !el.disabled; }})()"
    );
    page.evaluate(js)
        .await
        .ok()
        .and_then(|v| v.into_value::<bool>().ok())
        .unwrap_or(false)
}

/// Get an attribute value from an element matching `selector`.
#[allow(dead_code)]
pub async fn element_attribute(page: &Page, selector: &str, attr: &str) -> Option<String> {
    let escaped_sel = selector.replace('\\', "\\\\").replace('\'', "\\'");
    let escaped_attr = attr.replace('\\', "\\\\").replace('\'', "\\'");
    let js = format!(
        "(() => {{ const el = document.querySelector('{escaped_sel}'); \
         return el ? el.getAttribute('{escaped_attr}') : null; }})()"
    );
    page.evaluate(js)
        .await
        .ok()
        .and_then(|v| v.into_value::<serde_json::Value>().ok())
        .and_then(|v| v.as_str().map(String::from))
}
