//! Chromium browser test helpers — launch, teardown, element waits.
//!
//! Each integration test creates an isolated [`TestContext`] via [`setup`],
//! runs assertions, then cleans up via [`teardown`]. Browsers run headless.
//! The `base_url` points to the example server under test (defaults to
//! `http://localhost:3000`).

// Each e2e-tests/tests/*.rs binary compiles its own copy; not every helper is
// used by every binary, so suppressing dead_code is handled via per-function
// annotations rather than a module-level override.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::Page;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use e2e_tests::base_url;

/// Holds the browser, page, base URL, and profile directory for a single test.
///
#[derive(Debug)]
pub struct TestContext {
    pub browser: Browser,
    pub page: Page,
    /// Default base URL derived from the `BASE_URL` env var. Set during
    /// [`setup`] but currently only consumed via the explicit env URL; kept
    /// on the context for future assertions and diagnostic output.
    #[allow(
        dead_code,
        reason = "Currently set but not read; preserved for test diagnostics and future assertions."
    )]
    pub base_url: String,
    pub profile_dir: PathBuf,
    /// Token fired by [`teardown`] to stop the chromiumoxide handler task.
    pub shutdown: CancellationToken,
}

/// Generate a unique Chromium user-data-dir path using PID, nanos-since-epoch,
/// and an atomic counter.  The counter ensures uniqueness even when two threads
/// call this at the same monotonic instant.
#[allow(
    dead_code,
    reason = "Each tests/*.rs binary compiles its own copy of this module, so helpers may be unused in a given test target."
)]
fn unique_profile_dir() -> Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock must be after Unix epoch")?
        .as_nanos();
    let dir = format!(
        "/tmp/chromiumoxide-{pid}-{nanos}-{n}",
        pid = std::process::id(),
    );
    Ok(PathBuf::from(dir))
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
/// The chromiumoxide handler runs in a background [`tokio::task`] that races
/// against `ctx.shutdown`. [`teardown`] fires the token so the task exits
/// promptly instead of leaking for the rest of the test process when the
/// browser fails to close cleanly.
///
/// # Errors
/// Returns an error if the browser cannot launch, the page cannot be created,
/// or the profile directory cannot be created.
#[allow(
    dead_code,
    reason = "Each tests/*.rs binary compiles its own copy of this module, so helpers may be unused in a given test target."
)]
pub async fn setup() -> Result<TestContext> {
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

    let profile_dir = unique_profile_dir()?;
    std::fs::create_dir_all(&profile_dir).context("failed to create Chromium profile dir")?;

    let mut builder = BrowserConfig::builder()
        .user_data_dir(profile_dir.clone())
        .no_sandbox();
    if let Some(chrome_path) = chrome_path {
        builder = builder.chrome_executable(chrome_path);
    }
    let config = builder.build().map_err(|e| anyhow!("failed to build BrowserConfig: {e}"))?;

    let (browser, mut handler) = Browser::launch(config)
        .await
        .context("failed to launch Chromium browser")?;

    // Spawn the handler task — REQUIRED, otherwise the browser hangs.
    // Bound by `shutdown` so a failed `browser.close()` does not leak the
    // task for the rest of the test process.
    let shutdown = CancellationToken::new();
    let handler_token = shutdown.child_token();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = handler_token.cancelled() => break,
                event = handler.next() => {
                    if event.is_none() { break; }
                }
            }
        }
    });

    // Create a page with about:blank then navigate in each test.
    let page = browser
        .new_page("about:blank")
        .await
        .context("failed to create new page")?;

    Ok(TestContext {
        browser,
        page,
        base_url: base_url(None),
        profile_dir,
        shutdown,
    })
}

/// Tear down a [`TestContext`] by closing the page and browser in reverse
/// creation order. Cleanup errors are logged via `eprintln!` (stderr,
/// captured by the test harness per-test and displayed only on failure)
/// but never mask the test assertion that already ran.
#[allow(
    dead_code,
    reason = "Each tests/*.rs binary compiles its own copy of this module, so helpers may be unused in a given test target."
)]
pub async fn teardown(ctx: TestContext) {
    let TestContext {
        mut browser,
        page,
        profile_dir,
        shutdown,
        ..
    } = ctx;
    if let Err(e) = page.close().await {
        eprintln!("failed to close Chromium page during teardown: {e}");
    }
    if let Err(e) = browser.close().await {
        eprintln!(
            "failed to close Chromium browser during teardown; signalling handler shutdown: {e}"
        );
    }
    // Always fire the shutdown token so the handler task exits even if
    // `browser.close()` did not (e.g. the websocket is wedged).
    shutdown.cancel();
    // Remove the temporary profile directory.
    if let Err(e) = std::fs::remove_dir_all(&profile_dir) {
        eprintln!(
            "failed to remove Chromium profile dir {}: {e}",
            profile_dir.display()
        );
    }
}

/// Require the server at `url` to respond within 5 seconds.
///
/// # Errors
/// Returns an error if the server does not respond with a 2xx or 3xx status
/// before the timeout expires.
#[allow(
    dead_code,
    reason = "Each tests/*.rs binary compiles its own copy of this module, so helpers may be unused in a given test target."
)]
pub async fn require_server(url: &str) -> Result<()> {
    if !wait_for_server(url, Duration::from_secs(5)).await {
        bail!("server at {url} is not reachable");
    }
    Ok(())
}

// ──────  Element wait helpers  ──────

/// Poll `page.evaluate(expression)` until it returns `true` (as a boolean)
/// or the timeout elapses. Replacement for browser-framework wait helpers.
#[allow(
    dead_code,
    reason = "Each tests/*.rs binary compiles its own copy of this module, so helpers may be unused in a given test target."
)]
pub async fn wait_for_js_true(page: &Page, expression: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if page.evaluate(expression).await
            .ok()
            .and_then(|v| v.into_value::<bool>().ok())
            .unwrap_or(false)
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Poll `page.find_element(selector)` until it returns `Ok` or the timeout
/// elapses.  Returns the element if found.
#[allow(
    dead_code,
    reason = "Each tests/*.rs binary compiles its own copy of this module, so helpers may be unused in a given test target."
)]
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
#[allow(
    dead_code,
    reason = "Each tests/*.rs binary compiles its own copy of this module, so helpers may be unused in a given test target."
)]
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
