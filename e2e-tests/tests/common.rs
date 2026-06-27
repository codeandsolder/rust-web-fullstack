//! Test helpers, browser setup, and teardown for chromiumoxide E2E tests.
//!
//! Each test creates an isolated [`TestContext`] via [`setup`], runs assertions,
//! then cleans up via [`teardown`]. Browsers run headless. The `base_url` points
//! to the example server under test (defaults to `http://localhost:3000`).

use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::Page;
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Holds the browser, page, and base URL for a single test.
///
/// Fields may appear unused in some test binaries (each `tests/*.rs` file
/// compiles its own copy of this module); suppress the warning.
#[allow(dead_code)]
#[derive(Debug)]
pub struct TestContext {
    pub browser: Browser,
    pub page: Page,
    pub base_url: String,
}

/// Resolve the base URL — use `BASE_URL` env var or fall back to `http://localhost:3000`.
#[must_use]
pub fn base_url() -> String {
    std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

/// Poll a server until it responds with an HTTP 2xx / 3xx status, or the timeout
/// elapses.  Returns `true` if the server became healthy, `false` if the timeout
/// was reached.
///
/// The URL should be in `http://host:port/path` form.  The function sends a raw
/// HTTP/1.1 GET request via `TcpStream` (no extra dependencies).
pub async fn wait_for_server(url: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;

    let url = url.trim_end_matches('/');
    let addr = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("localhost:3000");
    let host = addr.split(':').next().unwrap_or("localhost");

    let request = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");

    while tokio::time::Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(addr).await
            && stream.write_all(request.as_bytes()).await.is_ok()
        {
            let mut buf = [0u8; 256];
            if stream.read(&mut buf).await.is_ok() {
                let response = String::from_utf8_lossy(&buf);
                // Accept any 2xx or 3xx status.
                if response.starts_with("HTTP/1.1 2")
                    || response.starts_with("HTTP/1.0 2")
                    || response.starts_with("HTTP/1.1 3")
                    || response.starts_with("HTTP/1.0 3")
                {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Initialise chromiumoxide, launch a headless Chromium browser, create a page,
/// and return a [`TestContext`].
///
/// # Panics
/// Panics if the browser cannot launch or the page cannot be created.
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

    // Use a unique user-data-dir per test to avoid SingletonLock conflicts
    // when cargo test runs in parallel.
    let profile_dir = std::env::temp_dir().join(format!(
        "chromiumoxide-test-{pid}-{ts}",
        pid = std::process::id(),
        ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));

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
        base_url: base_url(),
    }
}

/// Tear down a [`TestContext`] by closing the page and browser in reverse
/// creation order. Cleanup errors are printed for diagnostics but do not mask
/// the test assertion that already ran.
pub async fn teardown(ctx: TestContext) {
    let TestContext {
        mut browser, page, ..
    } = ctx;
    if let Err(e) = page.close().await {
        eprintln!("Failed to close Chromium page during teardown: {e}");
    }
    if let Err(e) = browser.close().await {
        eprintln!("Failed to close Chromium browser during teardown: {e}");
    }
}

/// Require the server at `url` to respond within 5 seconds.
///
/// # Panics
/// Panics if the server does not respond with a 2xx or 3xx status before the
/// timeout expires.
///
/// ```ignore
/// require_server(&base_url()).await;
/// ```
pub async fn require_server(url: &str) {
    assert!(
        wait_for_server(url, Duration::from_secs(5)).await,
        "server at {url} is not reachable"
    );
}

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

// ──────  Unit-testable helpers (no browser required)  ──────

/// Build a URL from a base and a path segment.  Handles leading slashes on
/// `path` and trailing slashes on `base`.
///
/// Used by `unit_tests.rs` (may appear unused in other test binaries).
#[allow(dead_code)]
#[must_use]
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}
