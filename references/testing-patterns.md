# Testing Patterns Reference

## Table of Contents
1. [Chrome DevTools MCP — Visual Exploration](#1-chrome-devtools-mcp--visual-exploration)
2. [chromiumoxide — Deterministic E2E Tests](#2-chromiumoxide--deterministic-e2e-tests)
3. [reqwest — HTTP-Only Tests](#3-reqwest--http-only-tests)
4. [SSE Verification Patterns](#4-sse-verification-patterns)
5. [Screenshot Diff Testing](#5-screenshot-diff-testing)
6. [CI Integration](#6-ci-integration)
7. [Test Strategy Decision Matrix](#7-test-strategy-decision-matrix)
8. [Pitfalls](#8-pitfalls)

---

## 1. Chrome DevTools MCP — Visual Exploration

### Available Tools

47 tools across 8 categories. The most useful for web testing:

| Tool | Purpose |
|------|---------|
| `navigate_page` | Go to URL, back, forward, reload; supports `initScript` |
| `take_snapshot` | Text-based DOM snapshot with unique `uid` per element |
| `take_screenshot` | Capture page or element as PNG/JPEG/WebP |
| `evaluate_script` | Execute arbitrary JavaScript, returns JSON-serializable result |
| `click` | Click element by snapshot `uid` |
| `fill` / `fill_form` | Fill form fields (use `fill_form` for multiple fields — much faster) |
| `press_key` | Key combinations (`Enter`, `Control+A`) |
| `wait_for` | Poll for text to appear on page |
| `list_console_messages` | All console messages since last navigation |
| `list_network_requests` | All network requests with filtering |
| `emulate` | Geolocation, user agent, viewport, dark mode, CPU/network throttling |
| `drag` / `hover` | Drag-and-drop, hover interactions |
| `lighthouse_audit` | Accessibility, SEO, best practices audit |

### Snapshot vs Screenshot

| Aspect | `take_snapshot` | `take_screenshot` |
|--------|----------------|-------------------|
| Output | JSON with `uid`, tag, text, attributes | PNG/JPEG/WebP image |
| Analyzable by | Any LLM (text) | Vision models only |
| Size | ~KB | ~100KB-MB |
| Interaction | **Required** for click, fill, hover | Reference only |
| Best for | DOM verification, element state | Visual layout, styling, responsive design |

**The snapshot is the primary interaction mechanism.** Take a snapshot first to get element `uid`s, then use those in `click`, `fill`, etc.

### Visual Testing Workflow

```
Step 1: navigate_page → http://localhost:3000
Step 2: take_snapshot → understand DOM structure
Step 3: For each feature:
   a. Interact (click, fill, etc.)
   b. take_snapshot → verify element state
   c. take_screenshot → visual confirmation (optional)
   d. list_console_messages → check for errors
Step 4: Document what you found
Step 5: Translate to chromiumoxide calls in a Rust test
```

### SSE Verification via Console Injection

```javascript
// Inject this to verify SSE is receiving data:
() => {
    const es = new EventSource('/api/events');
    es.onmessage = (e) => console.log('SSE_DATA:', e.data);
    es.addEventListener('search_results', (e) => {
        console.log('SSE_EVENT:', e.type, e.data);
        document.getElementById('live-output').textContent = e.data;
    });
    es.onerror = (e) => console.error('SSE_ERROR:', e);
    return 'SSE listener attached';
}
```

Then:
```
wait_for → ["SSE_DATA:"]  // confirm data arrived
list_console_messages → filter for "SSE_DATA"  // verify content
```

### When to Use Chrome MCP

- **"Show me what this looks like"** — layout, styling, responsive behavior
- **SSE debugging** — console message capture, network request inspection
- **Quick smoke tests** — navigate, click, check console for errors
- **Accessibility audit** — lighthouse_audit
- **Performance profiling** — performance_start_trace
- **Ad-hoc exploration** — "what does the settings page look like on mobile?"

### When NOT to Use Chrome MCP

- **CI pipeline** — not designed for it, requires LLM, non-deterministic
- **Cross-browser testing** — Chrome/Chromium only
- **Parallel test execution** — sequential by default
- **Precise DOM assertions** — no `expect(locator).to_have_count(3)`
- **Network mocking** — read-only inspection, no `page.route()`

---

## 2. chromiumoxide — Deterministic E2E Tests

### Why chromiumoxide over playwright-rs

The two available Rust ports of Playwright are both broken:

| Crate | Status | Why it fails |
|-------|--------|-------------|
| `octaltree/playwright-rust` | Abandoned (2021) | Bundles Playwright 1.11 with Chromium 90; modern Chromium speaks incompatible CDP |
| `padamson/playwright-rs` | Broken | Frame-channel RPC hangs on every `page.goto()` |
| `chromiumoxide 0.9` | **Works** | Raw CDP, no Node.js, no driver bundle, actively maintained |

### Installation

```toml
# Cargo.toml (e2e-tests crate only — never add to prod crates)
[dependencies]
# `default-features = false` keeps the tokio version compatible with the
# workspace pin; `bytes` is required for `Browser::launch(...)`.
chromiumoxide = { version = "0.9", default-features = false, features = ["bytes"] }
futures = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```bash
# Install Chromium via Playwright (chromiumoxide reuses these binaries)
npx playwright install chromium
# Or set `CHROME_PATH` in your shell — `setup()` reads it and passes the
# path to `BrowserConfig::chrome_executable(...)`. (chromiumoxide's own
# fallback `CHROME` env var is not used by this helper.)
```

### Browser Launch (with unique profile dir)

```rust
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

// CRITICAL: unique user_data_dir per test — see Pitfall #1
fn unique_profile_dir() -> std::path::PathBuf {
    // Per-process counter + nanos-since-epoch ensures uniqueness even when
    // two threads/tests call this at the same monotonic instant.
    // (Plain `Instant::now().elapsed()` would return ~0 because the Instant
    // was just created — a real correctness bug.)
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_nanos();
    let dir = format!("/tmp/chromiumoxide-{pid}-{nanos}-{n}", pid = std::process::id());
    std::path::PathBuf::from(dir)
}

#[tokio::test]
async fn test_ssr_page_renders() {
    let profile_dir = unique_profile_dir();
    std::fs::create_dir_all(&profile_dir)
        .expect("failed to create Chromium profile dir");

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .user_data_dir(profile_dir.clone())
            // No `.headless_mode(...)` needed — chromiumoxide 0.9 default
            // is already headless. `HeadlessMode` IS exported at
            // `chromiumoxide::browser::HeadlessMode`, but is only needed
            // for non-default headless configurations.
            .build()
    ).await.expect("failed to launch Chromium");

    // Pump CDP events in background — without this, the browser hangs.
    // (Verified against chromiumoxide 0.9 README; lifecycle driven by
    // `Browser::close()` signaling the handler channel — no CancellationToken
    // needed here.)
    tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await
        .expect("failed to create Chromium page");

    // Step 1: Navigate
    page.goto("http://localhost:3020/search").await
        .expect("failed to navigate to /search");

    // Step 2: Wait for hydration (poll for JS condition)
    let title_set = wait_for_js_true(
        &page,
        "() => document.querySelector('h1')?.innerText?.length > 0",
        Duration::from_secs(10),
    ).await;
    assert!(title_set, "Title did not render");

    // Step 3: Verify content
    let title: String = page.evaluate(
        "() => document.querySelector('h1')?.innerText ?? ''"
    ).await.unwrap().into_value().unwrap();
    assert!(title.contains("Search"));

    // Step 4: Fill form. `WebElement::type_str` does NOT reliably fire
    // the `input` event for Leptos's `bind:value` — use the IIFE pattern
    // that sets the value via the native setter and dispatches a single
    // bubbleable `input` event. See `e2e-tests/tests/live_search_test.rs`
    // `fill_search_input` for the canonical implementation.
    let query = "rust proxy";
    let value_json = serde_json::to_string(query).expect("query is always valid JSON");
    let script = format!(
        r#"(() => {{
            const el = document.querySelector('input[type="text"]');
            if (!el) throw new Error('search input not found');
            const setter = Object.getOwnPropertyDescriptor(
                window.HTMLInputElement.prototype, 'value'
            ).set;
            setter.call(el, {value_json});
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
        }})()"#
    );
    page.evaluate(script.as_str()).await.expect("failed to set input value");

    // Step 5: Click submit
    page.find_element("button[type='submit']").await.unwrap()
        .click().await.unwrap();

    // Step 6: Wait for results
    let has_results = wait_for_js_true(
        &page,
        "() => document.querySelectorAll('.result-item').length > 0",
        Duration::from_secs(10),
    ).await;
    assert!(has_results);

    let count: u32 = page.evaluate(
        "() => document.querySelectorAll('.result-item').length"
    ).await.unwrap().into_value().unwrap();
    assert!(count > 0);

    browser.close().await.unwrap();
    let _ = std::fs::remove_dir_all(&profile_dir);
}
```

### Helper Functions

```rust
use chromiumoxide::Page;
use std::time::{Duration, Instant};

/// Poll a JS expression until it returns true or timeout.
pub async fn wait_for_js_true(page: &Page, expr: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(v) = page.evaluate(expr).await {
            if v.into_value::<bool>().unwrap_or(false) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Require a service to be reachable before continuing a test.
pub async fn require_server(url: &str) {
    // `.expect()` takes a `&'static str` — it does NOT interpolate format
    // args. Use `.unwrap_or_else(|e| panic!(...))` when you need the URL
    // in the error message.
    let response = reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .unwrap_or_else(|e| panic!("required server at {url} is not reachable: {e}"));
    assert!(
        response.status().is_success(),
        "required server at {url} returned {}",
        response.status()
    );
}
```

### Console Event Capture

```rust
use futures::StreamExt;

// Subscribe to browser events
let mut events = browser.event_listener();

// In a background task:
tokio::spawn(async move {
    while let Some(event) = events.next().await {
        if let chromiumoxide::cdp::browser_protocol::log::Event::EntryAdded(entry) = event {
            println!("[console] {:?}", entry.entry);
        }
    }
});
```

### Screenshot Capture

```rust
use chromiumoxide::page::ScreenshotParams;

let bytes = page.screenshot(
    ScreenshotParams::builder()
        .format(chromiumoxide::page::ScreenshotFormat::Png)
        .full_page(true)
        .build()
).await.unwrap();

std::fs::write("/tmp/screenshot.png", bytes)?;
```

### Chrome Binary Selection (Playwright cache)

```rust
// Prefer the CHROME_PATH env var to avoid hardcoding paths
if let Ok(chrome_path) = std::env::var("CHROME_PATH") {
    builder = builder.chrome_executable(chrome_path);
}

// Or inline:
BrowserConfig::builder()
    .chrome_executable(std::path::PathBuf::from(
        "/path/to/chrome"
    ))
    .build()
```

Use `CHROME_PATH` env var rather than hardcoding. Common locations: Playwright cache,
system `/usr/bin/chromium`, or local download.
Verified stable: Chromium **1208** (Playwright 1.50 era). Chromium 1223+ has crashed on this host.

### Limitations

- **No CSS selector helpers** like Playwright's `expect(locator).to_have_text()` — write `page.evaluate("() => ...") yourself
- **No `page.route()`** — network interception requires CDP `Fetch.enable`, not exposed by chromiumoxide's high-level API
- **No built-in screenshot diff** — pair with `image` crate + `pixelmatch` for regression testing
- **Handler MUST be pumped** — spawning a background task that reads from `handler` is mandatory, otherwise the browser hangs

---

## 3. reqwest — HTTP-Only Tests

For tests that don't need a browser (JSON APIs, SSE stream reads, auth flows):

```rust
use reqwest::Client;

let client = Client::builder()
    .timeout(Duration::from_secs(5))
    .build()
    .unwrap();

// JSON API
let resp = client.get("http://localhost:3001/health").send().await.unwrap();
assert_eq!(resp.status(), 200);
let json: serde_json::Value = resp.json().await.unwrap();

// Auth login → JWT
let login = client.post("http://localhost:3001/auth/login")
    .json(&serde_json::json!({"user_id": "admin", "password": "admin"}))
    .send().await.unwrap();
let token: String = login.json::<serde_json::Value>().await.unwrap()
    ["token"].as_str().unwrap().to_string();

// Authenticated request
let auth = client.get("http://localhost:3001/admin/data")
    .bearer_auth(&token)
    .send().await.unwrap();
assert_eq!(auth.status(), 200);

// SSE stream read (lines)
let mut stream = client.get("http://localhost:3020/api/events")
    .send().await.unwrap()
    .bytes_stream();

use futures::StreamExt;
while let Some(chunk) = stream.next().await {
    let text = String::from_utf8_lossy(&chunk.unwrap());
    assert!(text.starts_with("data: ") || text.starts_with("event: "));
}
```

**Use reqwest when you can. Add chromiumoxide only when you need a real browser** (DOM inspection, JS execution, form interaction, hydration verification).

---

## 4. SSE Verification Patterns

### Pattern A: Wait for DOM Content (most reliable)

```rust
let populated = wait_for_js_true(
    &page,
    "() => document.querySelectorAll('#live-feed .item').length > 0",
    Duration::from_secs(15),
).await;
assert!(populated, "SSE did not populate #live-feed within 15s");
```

### Pattern B: Poll with JavaScript (flexible)

```rust
let connected = wait_for_js_true(
    &page,
    "() => document.querySelector('#sse-status')?.textContent === 'connected'",
    Duration::from_secs(10),
).await;
```

### Pattern C: Evaluate Loop (most control)

```rust
let start = Instant::now();
let timeout = Duration::from_secs(30);

loop {
    let connected: bool = page.evaluate(
        "() => !!document.querySelector('#live-feed')?.children.length"
    ).await.unwrap().into_value().unwrap();

    if connected { break; }
    if start.elapsed() > timeout {
        panic!("SSE data did not arrive within timeout");
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
}
```

### Pattern D: Inject + Listen (via evaluate_script in Chrome MCP)

```javascript
// Inject SSE listener that logs data to console
() => {
    const es = new EventSource('/api/events');
    es.onmessage = (e) => console.log('EVT:', e.data);
}
```
Then use `list_console_messages` to verify EVT messages arrived.

### Pattern E: End-to-End (insert DB row → assert SSE delivers it)

```rust
// 1. Trigger a NOTIFY by inserting into PostgreSQL
let pool = sqlx::PgPool::connect("postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo")
    .await.unwrap();
sqlx::query("INSERT INTO search_results (title, url, snippet) VALUES ($1, $2, $3)")
    .bind("Live test result")
    .bind("https://example.com")
    .bind("triggered by e2e test")
    .execute(&pool).await.unwrap();

// 2. Browser should receive via SSE within seconds
let arrived = wait_for_js_true(
    &page,
    r#"() => document.body.innerText.includes('Live test result')"#,
    Duration::from_secs(5),
).await;
assert!(arrived, "SSE did not deliver new row within 5s");
```

---

## 5. Screenshot Diff Testing

### chromiumoxide Screenshot Diff

```rust
use chromiumoxide::page::ScreenshotParams;

// Capture baseline
let bytes = page.screenshot(
    ScreenshotParams::builder()
        .format(chromiumoxide::page::ScreenshotFormat::Png)
        .full_page(true)
        .build()
).await.unwrap();
std::fs::write("tests/baselines/homepage.png", bytes)?;

// Compare with image crate + pixelmatch (external diff needed)
```

### Visual Regression Flow

```
1. chromiumoxide: Capture baseline screenshot
2. Make changes
3. chromiumoxide: Capture current screenshot
4. External tool: pixelmatch baseline.png current.png diff.png
5. Assert: diff_pixels < threshold
```

---

## 6. CI Integration

### chromiumoxide CI Config

```bash
# Install Chromium browser binary
npx playwright install chromium

# Run tests
cargo nextest run --features integration --test-threads=2
                                     # (with unique user_data_dir per test)
```

### GitHub Actions Example

```yaml
- name: Install Chromium
  run: npx playwright install --with-deps chromium

- name: Build app
  run: cargo leptos build

- name: Run E2E tests
  run: |
    cargo run --bin live-search &
    sleep 3
    cargo nextest run --test e2e
```

### Forgejo / Gitea Actions Example

Same as GitHub Actions — the action runner images work for both.

---

## 7. Test Strategy Decision Matrix

| You Want To... | Use | Why |
|----------------|-----|-----|
| See how the app renders | Chrome MCP | Visual feedback, quick, no code |
| Verify SSE stream works | Chrome MCP then chromiumoxide | MCP to debug, chromiumoxide to regression-test |
| Check console for errors | Chrome MCP | Built-in console listing |
| Test form submits correctly | chromiumoxide | Real keyboard events, deterministic |
| Run 200 tests in CI | reqwest + chromiumoxide | Fast JSON + real browser when needed |
| Catch visual regressions | chromiumoxide + pixelmatch | Pixel-level comparison |
| Debug a memory leak | Chrome MCP | Heap snapshots, dominators |
| Cross-browser test | Skip | Rust web stack targets Chromium only |
| Performance audit | Chrome MCP | Lighthouse, trace recording |
| Test auth flows | reqwest | POST /auth/login → JWT works fine without browser |
| Test hydration | chromiumoxide | Only browser can verify WASM ran |
| "Just look at this page" | Chrome MCP | One command, visual result |

---

## 8. Pitfalls

1. **chromiumoxide SingletonLock collision**: Default `~/.cache/chromiumoxide-runner/SingletonLock` collides when tests run in parallel. Always set a unique `user_data_dir` per test (e.g. `<pid>-<nanos>`).
2. **Forgot to pump the handler**: `Browser::launch` returns a `(Browser, Handler)` pair. If you don't `tokio::spawn` a task that reads from `handler.next()`, the browser hangs after the first CDP message.
3. **No network mocking in chromiumoxide**: Must run against a real backend. Set up test databases with fixtures.
4. **Hydration timing**: Leptos pages stream during hydration. Don't assert DOM state in `page.goto` callback — wait for a JS condition via `wait_for_js_true` with a generous timeout (5–10s).
5. **Snapshot ≠ full DOM**: Chrome MCP snapshots are based on the accessibility tree. Some elements (decorative divs, CSS-only content) may be missing.
6. **SSE detection timing**: SSE events arrive asynchronously. Always use `wait_for_js_true` with generous timeouts, not fixed `tokio::time::sleep`.
7. **Headless vs headed**: Chrome in headless mode may render slightly differently. For visual diff tests, use headless consistently.
8. **Silent test skips via `check_server_or_skip()`**: Do not use helpers that return `false` and let tests `return`. Required dependencies should panic/assert with the actual status or error; optional slow tests should use `#[ignore]`.
9. **Sccache does not shrink `target/`**: sccache caches rustc output to `~/.cache/sccache`, but `target/debug/deps/` still grows because cargo needs linked artifacts + incremental state. Use `cargo clean` periodically.
10. **Chromium binary version drift**: Playwright 1.50 Chromium 1208 is stable on this host; newer Chromium 1223+ has crashed. Pin via `BrowserConfig::chrome_path()`.
11. **Browser launch in CI**: Playwright browsers need system dependencies (`libnss3`, `libnspr4`, etc.). Use `--with-deps` flag in CI.
