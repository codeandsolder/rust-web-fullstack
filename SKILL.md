---
name: rust-web-fullstack
description: Full-stack Rust web development with Leptos 0.8.x, PostgreSQL via sqlx, axum, SSE streaming, and LISTEN/NOTIFY live queries. Use this skill when building Rust web apps, connecting Leptos to databases, implementing live updates, working with SSR/CSR/hydration, setting up SSE endpoints, using sqlx PgListener, writing E2E tests with chromiumoxide, or doing visual testing with Chrome DevTools MCP. Trigger on mentions of Leptos, sqlx, axum, PostgreSQL, SSE, LISTEN/NOTIFY, live queries, SSR, CSR, hydration, cargo-leptos, LeptosRoutes, server functions, PgPool, PgListener, broadcast channel, EventSource, chromiumoxide, or full-stack Rust architecture.
---

# Rust Web Fullstack — Leptos + PostgreSQL + Axum

## Canonical Reference Implementation

This skill ships with a complete, runnable reference workspace next to it. Every pattern in this skill is implemented and verified in that code.

| Path | What it shows |
|------|---------------|
| `./live-search/src/main.rs` | Pattern 15 (full triad): `CancellationToken` + `JoinSet` + signal handler + `tokio::select!` for `axum::serve` shutdown |
| `./live-search/src/db.rs::run_pg_listener` | Critical Rule 10 + Pitfall 14: `PgListener::recv()` raced against cancellation, with cancellable backoff sleep |
| `./gateway/src/main.rs` | Same Pattern 15 triad (no `JoinSet` needed — single-task shutdown) |
| `./gateway/src/module.rs::ServiceHealthError` | `#[non_exhaustive]` + `#[must_use]` + doc comment design pattern |
| `./gateway.Dockerfile` + `./live-search.Dockerfile` + `./docker-compose.yml` | Multi-stage Leptos build with `cargo-leptos`, runtime slim image, Postgres + pgAdmin + Chromium |
| `./e2e-tests/` | chromiumoxide-based Playwright replacement for browser-driven E2E |
| `./.woodpecker.yml` | Edition 2024 + Rust 1.94 + `--all-targets` + strict clippy with `-D warnings` |
| `./Cargo.toml` | Workspace Edition 2024 with strict `[workspace.lints]` table |

Build it yourself:

```bash
cd ~/.config/opencode/skills/rust-web-fullstack   # canonical location
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo leptos build                              # SSR + hydrate
```

Or via the symlink at `~/projects/rust-web-fullstack` (alias for the same directory).

## Quick Reference

### Crate Versions (June 2026)

Last verified against the canonical `Cargo.lock` in this directory on 2026-06-27.

| Crate | Version | Features | Notes |
|-------|---------|----------|-------|
| `leptos` | 0.8 | `csr`, `ssr`, `hydrate` — mutually exclusive per build target | 0.9 is alpha; stay on 0.8.x |
| `leptos_axum` | 0.8 | SSR integration with axum | Doubles `/api` prefix on server fns — see Pitfall 9 |
| `sqlx` | 0.9 | `postgres`, `runtime-tokio`, `tls-rustls`, `json`, `macros`, `migrate` | |
| `axum` | 0.8 | `json` | |
| `tokio` | 1 | `full` | |
| `tower-http` | 0.7 | `fs`, `trace` | `fs` is required for `ServeDir` |
| `jsonwebtoken` | 10 | **MUST** set `features = ["rust_crypto"]` or `["aws_lc_rs"]` | 10.x panics without explicit crypto provider — see Pitfall 10 |
| `reqwest` | 0.13 | `rustls` (renamed from 0.12's `rustls-tls`), `json` | |
| `chromiumoxide` | 0.9 | Direct CDP, no Node.js | Replaces abandoned playwright-rs |

### Architecture Decision Tree

```
Starting a Rust web project?
├── SEO/initial-load critical? → use SSR (hydrated)
│   └── axum + leptos_axum::render_app_to_stream_with_context
├── Internal tool / dashboard? → use CSR
│   └── leptos::mount_to_body
├── Need live updates? → LISTEN/NOTIFY + broadcast + SSE
│   └── sqlx::PgListener → tokio::sync::broadcast → axum SSE → EventSource
├── Need database? → PostgreSQL + sqlx
│   ├── Regular queries → PgPool
│   └── Live notifications → PgListener (borrows 1 connection from pool)
└── Need forms? → ServerAction + ActionForm (progressive enhancement)
```

### Critical Rules

1. **Feature flags are mutually exclusive per build target**: `csr`, `ssr`, `hydrate` cannot coexist
2. **Leptos 0.8 Action state**: use `action.value()` not `action.input()` — `input()` is `Some` only while pending; `value()` persists after completion. Required for "No results found." UI to display after submit.
3. **PgListener consumes 1 connection from the pool**: Budget for it in `max_connections`
4. **`render_app_to_stream_with_context` creates a fresh reactive tree per request**: Context injection is the standard way to share state
5. **SSE auto-headers**: axum's `Sse::new(stream)` automatically sets `Content-Type: text/event-stream` and `Cache-Control: no-cache`
6. **Server fn path doubling**: `leptos_axum::handle_server_fns` mounted at `/api/*fn_name` will register the route at `/api/api/search` when your server fn macro is configured with `endpoint = "/api/search"`. Fix: use a catch-all handler that tries both — see Pattern 9 below.
7. **Static files for hydration**: SSR pages load WASM via `/pkg/{crate}.js` and `/pkg/{crate}_bg.wasm`. You MUST mount `tower_http::services::ServeDir::new("./pkg")` (relative to server CWD) before hydration works. The Leptos build writes these to `./pkg` next to your `Cargo.toml` during `cargo leptos build`.
8. **chromiumoxide SingletonLock**: Every test that spawns a browser MUST use a unique `user_data_dir` (e.g. `<pid>-<nanos>`). Default `~/.cache/chromiumoxide-runner/SingletonLock` collides when tests run in parallel.
9. **Integration tests must fail visibly**: if a required service, browser, database, fixture, or SSE event is missing, panic/assert with the actual status or error. Use `#[ignore]` for intentionally optional slow tests; do not return early and report success.
10. **Background tasks need structured-concurrency wiring**: `pg_listener_task` and any other long-running `tokio::spawn`'d task MUST accept a `CancellationToken` and race its primary await against `shutdown.cancelled()` via `tokio::select!`. Dropping a `JoinHandle` does not cancel — only `token.cancel()` cooperatively stops the task. See Pattern 15.

---

## Reference Files

Load these as needed for deep patterns:

| File | When to Load | Content |
|------|-------------|---------|
| `references/leptos-patterns.md` | Writing Leptos components, SSR setup, forms, auth | Full Leptos 0.8.x cookbook (170 rules) |
| `references/postgres-patterns.md` | Database schema, sqlx usage, LISTEN/NOTIFY | PostgreSQL + sqlx patterns |
| `references/axum-patterns.md` | Routing, SSE, middleware | Axum 0.8 patterns |
| `references/testing-patterns.md` | Writing tests, visual testing, CI | Chrome MCP + chromiumoxide workflows |
| `references/architecture-patterns.md` | Multi-service gateway, shared crates | Architecture patterns from warpproxy/proxytest/searxrs2 |

---

## Core Patterns (Keep in SKILL.md)

### Pattern 1: Leptos SSR + Axum + PostgreSQL

```rust
// main.rs — Server binary (features = ["ssr"])
use axum::Router;
use anyhow::Context;
use leptos::*;
use leptos_axum::{generate_route_list, LeptosRoutes, handle_server_fns};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect("postgresql://localhost/mydb")
        .await
        .context("failed to connect to PostgreSQL")?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("failed to run database migrations")?;

    let conf = get_configuration(None).context("failed to read Leptos configuration")?;
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let routes = generate_route_list(App);

    let app = Router::new()
        .route("/api/*fn_name", get(handle_server_fns))
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .layer(TraceLayer::new_for_http())
        .with_state(leptos_options);

    // Inject pool into SSR context so server functions can use it
    let app_fn = move |leptos_options| {
        let app = App;
        leptos_axum::render_app_to_stream_with_context(
            move || {
                provide_context(pool.clone());
            },
            move || app,
        )(leptos_options)
    };

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind listener on {addr}"))?;
    axum::serve(listener, app.into_make_service())
        .await
        .context("server exited with an error")?;
    Ok(())
}

// Fallback for client-side: leptos::mount_to_body(App)
// #[cfg(feature = "hydrate")] or #[cfg(feature = "csr")]
```

### Pattern 2: Live Updates via LISTEN/NOTIFY → broadcast → SSE

```
┌──────────────┐    LISTEN/NOTIFY    ┌──────────────┐   broadcast   ┌──────────────┐
│ PostgreSQL    │ ────────────────── │ PgListener    │ ──────────── │ SSE Handler   │
│               │   NOTIFY channel   │ (sqlx)        │   tx.send()  │ (axum)        │
└──────────────┘                    └──────────────┘              └──────┬───────┘
                                                                         │
                                                                  text/event-stream
                                                                         │
                                                                  ┌──────▼───────┐
                                                                  │ Leptos Client │
                                                                  │ EventSource   │
                                                                  │ → ReadSignal   │
                                                                  └──────────────┘
```

**Server side (axum handler + PgListener)**:

```rust
use tokio::sync::broadcast;
use axum::response::sse::{Event, KeepAlive, Sse};
use sqlx::postgres::PgListener;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<Event>,
    pool: sqlx::PgPool,
}

async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tx.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|r| async move { r.ok() });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn pg_listener_task(
    pool: sqlx::PgPool,
    tx: broadcast::Sender<Event>,
    shutdown: CancellationToken,
) {
    let mut listener = match PgListener::connect_with(&pool).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!("failed to connect PostgreSQL listener: {e}");
            return;
        }
    };
    if let Err(e) = listener.listen_all(vec!["search_results", "proxy_status"]).await {
        tracing::error!("failed to subscribe PostgreSQL listener: {e}");
        return;
    }

    loop {
        tokio::select! {
            notification = listener.recv() => {
                let notification = match notification {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::error!("PostgreSQL listener receive failed: {e}");
                        break;
                    }
                };
                let event = Event::default()
                    .event(notification.channel())
                    .data(notification.payload());
                if let Err(e) = tx.send(event) {
                    tracing::debug!("notification had no SSE receivers: {e}");
                }
            }
            _ = shutdown.cancelled() => {
                tracing::info!("PgListener shutting down");
                break;
            }
        }
    }
}
```

**Client side (Leptos component consuming SSE)**:

```rust
use leptos::*;
use gloo_net::eventsource::futures::EventSource;
use futures::StreamExt;

fn live_feed() -> impl IntoView {
    let (data, set_data) = signal(String::new());

    // SSE subscription (CSR/hydrate only)
    #[cfg(not(feature = "ssr"))]
    {
        match EventSource::new("/api/events") {
            Ok(mut es) => {
                match es.subscribe("search_results") {
                    Ok(mut stream) => {
                        spawn_local(async move {
                            while let Some(Ok(msg)) = stream.next().await {
                                if let Some(text) = msg.data().as_string() {
                                    set_data.set(text);
                                } else {
                                    leptos::logging::warn!("SSE message had non-string data");
                                }
                            }
                        });
                        on_cleanup(move || es.close());
                    }
                    Err(e) => leptos::logging::error!("failed to subscribe to SSE: {e:?}"),
                }
            }
            Err(e) => leptos::logging::error!("failed to open SSE connection: {e:?}"),
        }
    }

    view! { <div id="live-data">{data}</div> }
}
```

### Pattern 3: PostgreSQL FTS with tsvector/tsquery

```rust
// Schema (in migration)
sqlx::query(
    "CREATE TABLE search_results (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        title TEXT NOT NULL,
        body TEXT NOT NULL,
        fts tsvector GENERATED ALWAYS AS (
            setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
            setweight(to_tsvector('english', coalesce(body, '')), 'B')
        ) STORED,
        created_at TIMESTAMPTZ DEFAULT now()
    )"
).execute(&pool).await?;

sqlx::query("CREATE INDEX idx_fts ON search_results USING GIN(fts)").execute(&pool).await?;

// Query with BM25-like ranking via ts_rank
sqlx::query_as::<_, SearchResult>(
    "SELECT *, ts_rank(fts, query) AS rank
     FROM search_results, to_tsquery('english', $1) query
     WHERE fts @@ query
     ORDER BY rank DESC
     LIMIT 20"
).bind(query_string).fetch_all(&pool).await?;
```

### Pattern 4: E2E Test with chromiumoxide

```rust
// Cargo.toml deps (e2e-tests crate only):
//   chromiumoxide = "0.9"
//   reqwest = { version = "0.13", features = ["rustls", "json"] }
//   futures = "0.3"
//   tokio = { version = "1", features = ["macros", "rt-multi-thread"] }

use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;

#[tokio::test]
async fn test_sse_live_update() {
    // Unique profile dir per test — chromiumoxide uses a SingletonLock
    // that collides when tests run in parallel.
    let profile_dir = format!("/tmp/chromiumoxide-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be after Unix epoch")
            .as_nanos());
    std::fs::create_dir_all(&profile_dir).expect("failed to create Chromium profile dir");

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .user_data_dir(std::path::PathBuf::from(&profile_dir))
            .headless_mode(chromiumoxide::browser::HeadlessMode::True)
            .build()
    ).await.unwrap();

    // Pump CDP events in the background
    tokio::spawn(async move { while let Some(_) = handler.next().await {} });

    let page = browser.new_page("about:blank").await.unwrap();
    page.goto("http://localhost:3020").await.unwrap();

    // Wait for SSE to populate the DOM via JS evaluation
    let populated = wait_for_js_true(
        &page,
        "() => document.getElementById('live-data')?.innerText?.length > 0",
        Duration::from_secs(10),
    ).await;

    assert!(populated, "SSE did not populate #live-data within 10s");

    let text: String = page
        .evaluate("() => document.getElementById('live-data')?.innerText ?? ''")
        .await
        .expect("failed to evaluate live-data text")
        .into_value()
        .expect("live-data text was not a string");
    assert!(!text.is_empty(), "Expected non-empty SSE content");

    if let Err(e) = std::fs::remove_dir_all(&profile_dir) {
        eprintln!("failed to remove Chromium profile dir {profile_dir}: {e}");
    }
}

// Helper: poll a JS expression until true or timeout
async fn wait_for_js_true(
    page: &chromiumoxide::Page,
    expr: &str,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if let Ok(val) = page.evaluate(expr).await {
            if val.into_value::<bool>().unwrap_or(false) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}
```

### Pattern 5: JSONB Storage with Compile-Time Checking

```rust
use sqlx::types::Json;

#[derive(serde::Serialize, serde::Deserialize)]
struct SearchResult {
    url: String,
    snippet: String,
}

// Insert (Json<T> maps to JSONB by default)
sqlx::query!(
    "INSERT INTO search_results (id, data) VALUES ($1, $2)",
    Uuid::new_v4(),
    Json(&result) as _,  // `as _` skips type verification for the JSON field
)
.execute(&pool).await?;

// Query with type annotation for compile-time checking
let rows = sqlx::query_as!(
    Row,
    r#"SELECT id, data as "data: Json<SearchResult>", created_at FROM search_results"#
)
.fetch_all(&pool).await?;
```

### Pattern 6: Gateway with ServiceModule Trait

```rust
use futures::future::{BoxFuture, FutureExt};

#[derive(Debug, thiserror::Error)]
#[error("service unhealthy")]
struct ServiceHealthError {
    #[source]
    source: sqlx::Error,
}

// Each service implements this trait
trait ServiceModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn router(&self) -> Router<AppState>;
    fn health_check<'a>(
        &'a self,
        pool: &'a PgPool,
    ) -> BoxFuture<'a, Result<(), ServiceHealthError>>;
}

struct SearxRs2;
impl ServiceModule for SearxRs2 {
    fn name(&self) -> &'static str { "searxrs2" }
    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/search", get(search_handler))
            .route("/api/search/*fn_name", get(handle_server_fns))
    }
    fn health_check<'a>(
        &'a self,
        pool: &'a PgPool,
    ) -> BoxFuture<'a, Result<(), ServiceHealthError>> {
        async move {
            sqlx::query("SELECT 1")
                .execute(pool)
                .await
                .map_err(ServiceHealthError::from)
        }
        .boxed()
    }
}

// Compose all services
fn build_gateway() -> Router<AppState> {
    let services: Vec<Box<dyn ServiceModule>> = vec![
        Box::new(SearxRs2),
        Box::new(ProxyTest),
        Box::new(WarpProxy),
    ];

    let mut router = Router::new();
    for service in &services {
        router = router.nest(&format!("/{}", service.name()), service.router());
    }
    router
}
```

### Pattern 7: JavaScript-Driven SSE Detection (for Chrome DevTools MCP)

```javascript
// Inject this via chrome-devtools_evaluate_script to verify SSE is working
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

### Pattern 8: TTL Cleanup via pg_cron

```sql
-- Install pg_cron extension (once per DB)
CREATE EXTENSION IF NOT EXISTS pg_cron;

-- Schedule hourly cleanup of expired search results
SELECT cron.schedule(
    'cleanup-expired-results',
    '0 * * * *',  -- every hour
    $$DELETE FROM search_results WHERE created_at < now() - INTERVAL '30 days'$$
);
```

### Pattern 9: Server-Fn Catch-All Route (Leptos 0.8 Doubled-Prefix Bug)

`leptos_axum::handle_server_fns` registers server functions at the path
declared by their `endpoint = "..."` macro arg. When that arg starts with
`/api/`, the resulting route is `/api/api/<fn_name>` — clients calling
`/api/search` get 404. This is a known wart of Leptos 0.8's macro expansion.
Fix with a catch-all handler that tries both paths:

```rust
use axum::{routing::post, extract::Path, http::Uri};
use leptos_axum::handle_server_fns;

let app = Router::new()
    .route("/api/{*fn_name}", post(
        |Path(fn_name): Path<String>, uri: Uri, req: Request| async move {
            // Try the exact path first; fall back to doubled-prefix variant.
            let path = uri.path().to_string();
            handle_server_fns_with_path(path, req).await
        }
    ));

async fn handle_server_fns_with_path(
    path: String,
    req: Request,
) -> Response {
    // Try as-is, then with the doubled /api prefix stripped + re-added.
    let tried = path.clone();
    match handle_server_fns_internal(&path, &req).await {
        Ok(r) => r,
        Err(_) => {
            // Fallback: replace leading /api/ with /api/api/ if present
            let fallback = if path.starts_with("/api/")
                && !path.starts_with("/api/api/")
            {
                path.replacen("/api/", "/api/api/", 1)
            } else {
                tried
            };
            handle_server_fns_internal(&fallback, &req).await
                .unwrap_or_else(|_| (StatusCode::NOT_FOUND, "fn not found").into_response())
        }
    }
}
```

Or simpler: register the `handle_server_fns` route at `/api/{*fn_name}` and
also at `/api/api/{*fn_name}` so both variants work — accept the duplicate.

### Pattern 10: SSR + Hydration Setup (Same Crate as Both Bin & Lib)

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "live-search"
path = "src/main.rs"

[features]
hydrate = ["live-search/leptos/hydrate"]
ssr = ["live-search/leptos/ssr"]
```

```rust
// src/lib.rs
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn hydrate() {
    use crate::app::App;
    leptos::mount::hydrate_body(App);
}
```

```rust
// src/main.rs — server binary, serves SSR HTML + WASM bundle
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let conf = get_configuration(None).context("failed to read Leptos configuration")?;
    let leptos_options = conf.leptos_options;

    let app = Router::new()
        // Catch-all server-fn route (see Pattern 9)
        .route("/api/{*fn_name}", post(handle_server_fns))
        // Static WASM + JS bundle — MUST exist for hydration
        .nest_service("/pkg", tower_http::services::ServeDir::new("./pkg"))
        // Leptos page routes (returns SSR HTML)
        .leptos_routes(&leptos_options, routes, {
            let opts = leptos_options.clone();
            move || shell(opts.clone())
        })
        .layer(TraceLayer::new_for_http())
        .with_state(leptos_options);

    let listener = tokio::net::TcpListener::bind(&leptos_options.site_addr)
        .await
        .context("failed to bind SSR listener")?;
    axum::serve(listener, app.into_make_service())
        .await
        .context("SSR server exited with an error")?;
    Ok(())
}
```

```rust
// Server-only modules (DB pool, listener task) gated by feature
#[cfg(feature = "ssr")]
pub mod server {
    use sqlx::postgres::{PgPool, PgListener};
    // ...
}
```

### Pattern 11: Action.value() vs Action.input()

`Action::input()` returns `Option<Input>` — `Some` only while the action is
in-flight. Once it completes, `input()` returns `None` and any reactive
view that reads it disappears.

`Action::value()` returns `Option<Output>` — `Some` once the action
**completes** (success or error), persists for the lifetime of the action.

```rust
// WRONG: "No results found." vanishes the moment the request returns
view! {
    {move || search_action.input()
        .map(|_| "No results found.")
    }
}

// RIGHT: persists after action completes
view! {
    {move || match search_action.value() {
        Some(Ok(results)) if results.is_empty() => view! { <p>"No results found."</p> }.into_any(),
        Some(Ok(results)) => view! { <ul>{results}</ul> }.into_any(),
        Some(Err(e)) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
        None => view! { <p>"Type a query and submit"</p> }.into_any(),
    }}
}
```

### Pattern 12: chromiumoxide E2E Helpers

```rust
// tests/common.rs — shared helpers

use chromiumoxide::{Browser, BrowserConfig, Page};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub fn unique_profile_dir() -> std::path::PathBuf {
    // Per-process counter combined with nanos-since-epoch guarantees uniqueness
    // even when two threads/tests call this at the same monotonic instant.
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

pub async fn setup() -> TestContext {
    let dir = unique_profile_dir();
    std::fs::create_dir_all(&dir).expect("failed to create Chromium profile dir");

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .user_data_dir(dir.clone())
            .build()
    ).await.expect("failed to launch Chromium");

    // Pump CDP events in the background — without this the browser hangs.
    // (Verified against chromiumoxide 0.9 README pattern; no CancellationToken
    // needed — the handler lifecycle is driven by the Browser handle via
    // `browser.close()`, which signals the handler channel.)
    tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await
        .expect("failed to create Chromium page");

    TestContext { browser, page, profile_dir: dir, base_url: base_url() }
}

pub async fn teardown(ctx: TestContext) {
    // eprintln! is intentional: the Rust test harness captures stderr per-test
    // and only displays it on failure. A tracing subscriber is not initialized
    // in tests, so tracing::warn! would be silently dropped.
    if let Err(e) = ctx.browser.close().await {
        eprintln!("failed to close Chromium browser: {e}");
    }
    if let Err(e) = std::fs::remove_dir_all(&ctx.profile_dir) {
        eprintln!("failed to remove Chromium profile dir: {e}");
    }
}

pub async fn wait_for_js_true(page: &Page, expr: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(v) = page.evaluate(expr).await {
            if v.into_value::<bool>().unwrap_or(false) { return true; }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

// Fail fast when a required service isn't running.
pub async fn require_server(url: &str) {
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

### Pattern 13: chromiumoxide Chrome Binary Selection

chromiumoxide launches Chrome via the system's default Chrome binary
detection. On a system with both Playwright's Chromium and the system
Chrome installed, force a specific binary via `chromiumoxide::BrowserConfig`:

```rust
BrowserConfig::builder()
    .chrome_path(std::path::PathBuf::from(
        "/home/jan/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome"
    ))
    .build()
```

Verified stable: Chromium **1208** (Playwright 1.50 era).
Crashes observed: Chromium **1223** (Playwright 1.51+ on this host).

### Pattern 14: SSE JSON Injection in Rust Raw Strings

When using `format!()` to build SSE event payloads that include JSON, the
literal `{{` and `}}` escape sequences produce literal `{` and `}` — but
double braces from `{json_field}` placeholders also produce `{}`. To avoid
both pitfalls, use **raw string literals** (`r#"..."#`) and explicit
`replace()` for any interpolation:

```rust
// WRONG: format! doubles { and } for escape, breaks downstream JSON parsing
let payload = format!(r#"data: {{"query":"{q}","results":[]}}"#);

// RIGHT: raw string + explicit replacement
let payload = r#"data: {"query":"__QUERY__","results":[]}"#
    .replace("__QUERY__", &q);
```

Apply same rule to test JS strings — never `format!` JS source code.

### Pattern 15: Structured Concurrency Triad (CancellationToken + JoinSet + select!)

Wire `axum::serve`, `pg_listener_task`, and any other long-lived spawned task into a single shutdown signal so SIGINT/SIGTERM cleans up cooperatively. This satisfies the `async-cancellation-token` + `async-structured-concurrency` + `async-joinset-structured` rules from rust-skills.

```rust
// Cargo.toml
//   tokio-util = "0.7"

use std::time::Duration;
use tokio::{signal, task::JoinSet};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ... pool setup, app router build as in Pattern 1 ...

    let shutdown = CancellationToken::new();

    // Spawn a signal handler that fires the shutdown token on Ctrl+C / SIGTERM
    let signal_token = shutdown.clone();
    tokio::spawn(async move {
        let ctrl_c = async {
            signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
        };
        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();
        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
        tracing::info!("shutdown signal received");
        signal_token.cancel();
    });

    // Spawn pg_listener_task in a JoinSet with a child token
    let mut tasks = JoinSet::new();
    let listener_token = shutdown.child_token();
    tasks.spawn(pg_listener_task(pool.clone(), tx.clone(), listener_token));

    // Run axum::serve, racing against shutdown
    let listener = tokio::net::TcpListener::bind(&addr).await
        .with_context(|| format!("failed to bind listener on {addr}"))?;
    let server_token = shutdown.clone();
    let server = axum::serve(listener, app.into_make_service());
    tokio::select! {
        result = server => {
            result.context("axum server exited with an error")?;
        }
        _ = server_token.cancelled() => {
            tracing::info!("axum shutdown requested");
        }
    }

    // Drain remaining tasks with a grace period
    shutdown.cancel();
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        async { while tasks.join_next().await.is_some() {} }
    ).await;

    Ok(())
}
```

**Why this works**:
- `CancellationToken::cancel()` is observed by every clone and child token — `pg_listener_task`'s `tokio::select!` wakes up and breaks its loop.
- `JoinSet::join_next()` awaits task completion; tasks spawned on the set are aborted on drop (but we drain first via `timeout`).
- A second `shutdown.cancel()` after `axum::serve` returns is idempotent — safe to call even if the signal handler already fired.

---

## Common Pitfalls

1. **PgListener connection leak**: Always call `listener.listen()` BEFORE entering the recv loop; the connection is held for the listener's lifetime
2. **Broadcast channel overflow**: Default buffer is 256; lagging consumers should receive an explicit `stream_lagged`/diagnostic event and publishers should log `SendError` when no receivers exist
3. **Leptos SSR hangs**: If a server function never resolves, the SSR stream blocks indefinitely — use `.timeout()` on async operations
4. **JSONB in sqlx macros**: Use `as _` cast for Json<T> in `query!()` macros; otherwise the macro can't infer the type
5. **Feature flag conflicts**: `csr`, `ssr`, `hydrate` are mutually exclusive — use `[features]` section in Cargo.toml to enforce this with `skip_feature_sets`
6. **cross-origin SSE**: EventSource requires same-origin by default; use CORS headers or serve from same domain
7. **chromiumoxide user_data_dir collision**: Default `~/.cache/chromiumoxide-runner/SingletonLock` collides when tests run in parallel. Always set a unique `user_data_dir` per test (see Pattern 12).
8. **WASM hydration requires static serving**: SSR HTML references `/pkg/{crate}.js` and `/pkg/{crate}_bg.wasm`. Without `ServeDir::new("./pkg")` mounted on the router, the page renders but JavaScript never runs. Verify with `curl http://localhost:3020/pkg/live_search.js` returning 200.
9. **Server-fn 404 / doubled-prefix**: `endpoint = "/api/search"` + `handle_server_fns` mounted at `/api/*fn_name` → server fn is reachable only at `/api/api/search`. Either mount the route at `/api/api/*fn_name`, or use a catch-all handler that tries both (Pattern 9).
10. **jsonwebtoken 10 panics without crypto provider**: `jsonwebtoken = "10"` alone crashes the process on first `encode()`/`decode()` call with `Could not automatically determine the process-level CryptoProvider`. Fix with `features = ["rust_crypto"]` (pure Rust) or `["aws_lc_rs"]` (requires cmake/perl/nasm C toolchain).
11. **Silent test skips via `check_server_or_skip()`**: Do not use helpers that return `false` and let tests `return`. Required dependencies should panic/assert with the actual status or error; optional slow tests should use `#[ignore]`.
12. **Stale `target/debug/deps/` fingerprints**: Every Cargo.toml change creates new `.rlib` hashes (e.g. `libplaywright-*.rlib`, `libchromiumoxide-{hash}.rlib`). Cargo never garbage-collects. Use `cargo clean` periodically or `cargo-sweep` to reclaim disk. Sccache eliminates compile time but does NOT shrink `target/`.
13. **`sccache` is local-disk by default**: No `SCCACHE_*` env vars or `~/.config/sccache/config.toml` means `~/.cache/sccache` (local). For remote/distributed caching, set `SCCACHE_BUCKET` (S3) or `SCCACHE_REDIS` (Redis) explicitly.
14. **Background tasks missing CancellationToken wiring**: `tokio::spawn(pg_listener_task(pool, tx))` without a `CancellationToken` parameter cannot be cancelled — the task runs forever even when the server is shutting down. The `recv().await` future IS cancel-safe (sqlx 0.9 PgListener drops the TCP read cleanly), so wrap the loop in `tokio::select!` against `token.cancelled()`, then fire the token from a Ctrl+C/SIGTERM handler in `main()`. See Pattern 15.

---

## Test Strategy

| Phase | Tool | Purpose |
|-------|------|---------|
| Visual exploration | Chrome DevTools MCP | Screenshots, DOM snapshots, console errors |
| SSE verification | Chrome DevTools MCP | `list_console_messages` to confirm SSE data arrived |
| Deterministic CI | chromiumoxide 0.9 | `page.evaluate()`, `wait_for_js_true`, real browser assertions |
| HTTP-only CI | reqwest 0.13 | JSON API + SSE stream reads via `bytes_stream()` |
| Screenshot diff | chromiumoxide + image crate | Pixel-level regression detection |
| Performance | Chrome DevTools MCP | Lighthouse audits, trace recording |

**Golden rule**: Chrome MCP for exploration, chromiumoxide for CI. Never put Chrome MCP in a CI pipeline (requires manual interaction).

**Why not playwright-rs**: The two available Rust ports are both broken —
`octaltree/playwright-rust` bundles Playwright 1.11 (2021) with Chromium
90, which speaks an incompatible CDP dialect; `padamson/playwright-rs`
hits a Frame-channel RPC hang on every `page.goto()`. Chromiumoxide 0.9
uses raw CDP directly (no Node.js, no driver bundle) and is actively
maintained. Pin Chromium to the 1208 build if newer versions crash on
your host (Pattern 13).

---

## Bundle of Patterns (for AI model loading)

When you need deeper patterns, load the relevant reference file:

- **Writing Leptos code?** → `references/leptos-patterns.md`
- **Setting up PostgreSQL?** → `references/postgres-patterns.md`
- **Configuring axum routes?** → `references/axum-patterns.md`
- **Writing tests?** → `references/testing-patterns.md`
- **Designing architecture?** → `references/architecture-patterns.md`
