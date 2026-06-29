---
name: rust-web-fullstack
description: Full-stack Rust web development with Leptos 0.8.x, PostgreSQL via sqlx, axum, SSE streaming, and LISTEN/NOTIFY live queries. Use this skill when building Rust web apps, connecting Leptos to databases, implementing live updates, working with SSR/CSR/hydration, setting up SSE endpoints, using sqlx PgListener, writing E2E tests with chromiumoxide, or doing visual testing with Chrome DevTools MCP. Trigger on mentions of Leptos, sqlx, axum, tower-http, tower, axum middleware, PostgreSQL, SSE, LISTEN/NOTIFY, live queries, pg_notify, SSR, CSR, hydration, cargo-leptos, LeptosRoutes, server functions, PgPool, PgListener, broadcast channel, EventSource, chromiumoxide, JWT, session, HttpOnly, WASM, wasm-bindgen, hydrate_body, Cargo workspace, [workspace.lints], Edition 2024, graceful shutdown, signal handling, SIGTERM, tracing_subscriber, RUST_LOG, EnvFilter, tracing spans, #[instrument], nextest, insta, rstest, mockall, criterion, proptest, CancellationToken, JoinSet, tokio::select!, or full-stack Rust architecture.
---

# Rust Web Fullstack — Leptos + PostgreSQL + Axum

## Canonical Reference Implementation

This skill ships with a complete, runnable reference workspace next to it. Every pattern in this skill is implemented and verified in that code.

| Path | What it shows |
|------|---------------|
| `./live-search/src/main.rs` | Pattern 15 (full triad): `CancellationToken` + `JoinSet` + signal handler + `tokio::select!` for `axum::serve` shutdown |
| `./live-search/src/db.rs::run_pg_listener` | Critical Rule 10 + Pitfall 14: `PgListener::recv()` raced against cancellation, with cancellable backoff sleep |
| `./gateway/src/main.rs` | Pattern 15's shutdown primitive only (no spawned tasks → no `JoinSet` / `CancellationToken` required) |
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

Last verified against the canonical `Cargo.lock` in this directory on 2026-06-29.

| Crate | Version | Features | Notes |
|-------|---------|----------|-------|
| `leptos` | 0.8 | `csr`, `ssr`, `hydrate` — mutually exclusive per build target | 0.9 is alpha; stay on 0.8.x |
| `leptos_axum` | 0.8 | SSR integration with axum | Doubles `/api` prefix on server fns — see Pitfall 9 |
| `sqlx` | 0.9 | `postgres`, `runtime-tokio`, `tls-rustls`, `json`, `macros`, `migrate` | Canonical workspace uses *runtime* queries only (`sqlx::query_as::<_, T>(…)`); compile-time `query!` requires `cargo sqlx prepare` + `.sqlx/` cache |
| `axum` | 0.8 | `json` | |
| `tokio` | 1 | `full` | `JoinSet` and `tokio::time::Instant` come from `tokio` directly |
| `tokio-util` | 0.7 | (no feature needed) | `CancellationToken` lives in `tokio_util::sync` and is reachable without any feature gate |
| `tower-http` | 0.7 | per-crate: `trace` is workspace-default; live-search adds `fs` (for `ServeDir`); gateway adds `cors` | `fs` is required only by crates that mount `ServeDir` |
| `jsonwebtoken` | 10 | **MUST** set `features = ["rust_crypto"]` or `["aws_lc_rs"]` | 10.x panics without explicit crypto provider — see Pitfall 10 |
| `reqwest` | 0.13 | `default-features = false`, `rustls`, `json`, `stream` | `default-features = false` avoids the `native-tls` conflict with `rustls`; `stream` enables `bytes_stream()` for SSE reading |
| `chromiumoxide` | 0.9 | `default-features = false`, `bytes` | `default-features = false` keeps the tokio version compatible with the workspace pin; `bytes` is required for `Browser::launch(...)` |
| `gloo-net` | 0.7 | `eventsource` | Client-side SSE reader |
| `gloo-timers` | 0.3 | `futures` | `gloo_timers::future::sleep` requires the `futures` feature |
| `tracing` | 0.1 | (default) | Structured logging — never `println!` or `log` |
| `tracing-subscriber` | 0.3 | `env-filter`, `fmt` | `env-filter` required to read `RUST_LOG`; install once in `main` (Pattern 0) |

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

### When NOT to use this skill

- **You need React/Vue/Svelte interoperability**: Leptos is Rust-only; no JS interop for 3rd party components
- **Your team is not familiar with Rust**: The fullstack Rust learning curve is steep — consider this only if the team already ships Rust
- **You need quick prototyping**: Use a managed backend (Supabase, Convex) and a JS frontend for MVP speed
- **Your app is read-heavy with no real-time needs**: A simpler SSR-only setup (e.g. axum + maud/askama) avoids the complexity of hydration, WASM, and SSE
- **You need mobile rendering**: Leptos targets the web; use Tauri + Leptos for desktop, but not for mobile-first
- **You need extensive 3rd-party JS ecosystem**: If your app requires many client-side JS libraries without WASM wrappers, stick with a JS framework

### Critical Rules

1. **Feature flags are mutually exclusive per build target**: `csr`, `ssr`, `hydrate` cannot coexist
2. **Leptos 0.8 Action state**: use `action.value()` (the action's result) not `action.input()` (the dispatched input) when rendering post-action result UI ("No results found.", error banner, success state). Both `input()` and `value()` persist for the lifetime of the `Action`; they differ in what they hold, not in whether they survive completion. See Pattern 11.
3. **PgListener consumes 1 connection from the pool**: Budget for it in `max_connections`
4. **`render_app_to_stream_with_context` creates a fresh reactive tree per request**: Context injection is the standard way to share state
5. **SSE auto-headers**: axum's `Sse::new(stream)` automatically sets `Content-Type: text/event-stream` and `Cache-Control: no-cache`
6. **Server fn path doubling**: `leptos_axum::handle_server_fns` mounted at `/api/*fn_name` will register the route at `/api/api/search` when your server fn macro is configured with `endpoint = "/api/search"`. Fix: use a catch-all handler that tries both — see Pattern 9 below.
7. **Static files for hydration**: SSR pages load WASM via `/pkg/{crate}.js` and `/pkg/{crate}_bg.wasm`. You MUST mount `tower_http::services::ServeDir::new("./pkg")` (relative to server CWD) before hydration works. The Leptos build writes these to `./pkg` next to your `Cargo.toml` during `cargo leptos build`.
8. **chromiumoxide SingletonLock**: Every test that spawns a browser MUST use a unique `user_data_dir` (e.g. `<pid>-<nanos>`). Default `~/.cache/chromiumoxide-runner/SingletonLock` collides when tests run in parallel.
9. **Integration tests must fail visibly**: if a required service, browser, database, fixture, or SSE event is missing, panic/assert with the actual status or error. Use `#[ignore]` for intentionally optional slow tests; do not return early and report success.
10. **Background tasks need structured-concurrency wiring**: `pg_listener_task` and any other long-running `tokio::spawn`'d task MUST accept a `CancellationToken` and race its primary await against `shutdown.cancelled()` via `tokio::select!`. Dropping a `JoinHandle` does not cancel — only `token.cancel()` cooperatively stops the task. See Pattern 15.
    *If your binary has no `tokio::spawn` calls (the gateway, for example), `with_graceful_shutdown(graceful_shutdown_signal())` is sufficient and no `CancellationToken` is needed — `Pattern 15` is still relevant as a reference, but only its shutdown primitive applies.*

---

## Workspace Setup

### `[workspace.lints]` Table

The skill advertises "strict clippy with `-D warnings`" (line 20). Here is the
canonical table to copy into your root `Cargo.toml`. Every code sample in this
skill compiles under these rules.

```toml
[workspace.lints.rust]
unsafe_code = "deny"
rust_2024_compatibility = { level = "deny", priority = -1 }
missing_debug_implementations = "warn"

[workspace.lints.clippy]
pedantic = { level = "deny", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
nursery = { level = "warn", priority = -1 }
too_long_first_doc_paragraph = "allow"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true

[profile.dev.package."*"]
opt-level = 3   # compile dependencies with optimisations even in dev
```

Rules:
- `#[expect(...)]` over `#[allow(...)]` so stale suppressions become visible when
  the lint no longer fires (`err-expect-not-allow`).
- Test crates override `unwrap_used = "allow"` and `expect_used = "allow"`
  because tests legitimately fail-fast.
- Never silence `panic`, `todo`, `unimplemented` — they are deliberately
  banned in non-test code.

### Tracing Subscriber Init

Every `tracing::info!` / `warn!` / `error!` call in this skill is a **silent
no-op** until a subscriber is installed. Add this at the top of every binary's
`main` (before any logging call):

```rust
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,hyper=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).with_thread_ids(false))
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();    // FIRST — before anything that can log
    // ... rest of main
}
```

Enable with `RUST_LOG=debug` (or `RUST_LOG=sqlx=debug,info`) at runtime. For
JSON output (downstream observability pipelines), swap `fmt::layer()` for
`fmt::layer().json()`.

### Structured Fields, Not Interpolation

Always record structured key-value fields, never string-interpolated values
(`obs-structured-fields`):

```rust
// RIGHT — fields are queryable in tracing-subscriber JSON / log aggregators
tracing::error!(error = %e, channel = "search_results",
    "PgListener recv failed; reconnecting");

// WRONG — the error message is buried in the formatted string
tracing::error!("PgListener recv failed: {e}");
```

Use `#[tracing::instrument]` for request-scoped context, and `skip` every
argument you don't want recorded (large pools, request bodies, secrets):

```rust
#[tracing::instrument(
    skip_all,
    fields(req_id = %Uuid::new_v4(), user_id = %req.user_id),
)]
pub async fn login_handler(
    State(state): State<GatewayState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    // `state` and `req` are NOT recorded — no Settings dump, no password leak
    state.rate_limiter.check(addr.ip()).await?;
    // ...
}
```

`skip_all` is the safe default. If you must record a field, list it explicitly:
`skip(state, req, fields(user_id = %req.user_id))`.

---

## Reference Files

Load these as needed for deep patterns:

| File | When to Load | Content |
|------|-------------|---------|
| `references/leptos-patterns.md` | Writing Leptos components, SSR setup, forms, auth | Leptos 0.8.x cookbook (~50 rules across 10 sections) |
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
use leptos_axum::{generate_route_list, LeptosRoutes};
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
        // Static assets FIRST — before leptos_routes so /pkg/* takes priority
        .nest_service("/pkg", tower_http::services::ServeDir::new("./pkg"))
        // Server function catch-all — see Pattern 9 for the custom handler
        .route("/api/{*fn_name}", axum::routing::any(server_fn_handler))
        // SSE endpoint
        .route("/api/events", axum::routing::get(sse_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(leptos_options.clone())
        // Leptos page routes last (catch-all within its domain)
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(fallback_handler);

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
    // Retry-with-backoff loop: never panic on transient DB unavailability.
    // See Pattern 15 (Structured Concurrency) for the wider shutdown triad.
    let mut backoff = Duration::from_millis(250);
    let max_backoff = Duration::from_secs(30);

    'connect: loop {
        let mut listener = tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                tracing::info!(component = "pg_listener", "cancelled before connect");
                return;
            }
            res = PgListener::connect_with(&pool) => match res {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(error = %e, backoff_ms = backoff.as_millis() as u64,
                        component = "pg_listener",
                        "PgListener connect failed; retrying");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                    continue 'connect;
                }
            },
        };

        if let Err(e) = listener
            .listen_all(vec!["search_results", "proxy_status"])
            .await
        {
            tracing::warn!(error = %e, component = "pg_listener",
                "PgListener listen_all failed; reconnecting");
            continue 'connect;
        }

        tracing::info!(component = "pg_listener", "connected and listening");
        backoff = Duration::from_millis(250); // reset on success

        loop {
            tokio::select! {
                biased;                    // shutdown wins ties
                _ = shutdown.cancelled() => {
                    tracing::info!(component = "pg_listener", "shutting down");
                    return;
                }
                notification = listener.recv() => {
                    match notification {
                        Ok(n) => {
                            let event = Event::default()
                                .event(n.channel())
                                .data(n.payload());
                            if let Err(e) = tx.send(event) {
                                tracing::debug!(error = %e,
                                    "notification had no SSE receivers");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, component = "pg_listener",
                                "recv failed; reconnecting");
                            continue 'connect;
                        }
                    }
                }
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

    // SSE subscription (WASM-only — gloo_net::eventsource has no SSR impl)
    #[cfg(target_arch = "wasm32")]
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
//   chromiumoxide = { version = "0.9", default-features = false, features = ["bytes"] }
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
            // No `.headless_mode(...)` call needed: the chromiumoxide 0.9 default
            // is already headless. Note that `HeadlessMode` itself is *not*
            // publicly re-exported by chromiumoxide 0.9 (issue #317), so any
            // explicit selector must use `.new_headless_mode()` instead.
            .build()
    )
    .await
    .expect("failed to launch Chromium for SSE test");

    // Pump CDP events in the background. The handler terminates when
    // browser.close() drops the underlying websocket.
    tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await
        .expect("failed to create Chromium page");
    page.goto("http://localhost:3000").await
        .expect("failed to navigate to live-search");

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
        tracing::warn!(
            profile_dir = %profile_dir,
            error = %e,
            "failed to remove Chromium profile dir"
        );
    }
}
```

> **Lint compatibility:** the canonical workspace sets
> `clippy::unwrap_used = "deny"` and `clippy::expect_used = "deny"` for
> production crates. Test crates relax these (`unwrap_used = "allow"`,
> `expect_used = "allow"` in `e2e-tests/Cargo.toml`) so `.unwrap()` /
> `.expect()` for fail-fast test setup are acceptable there. Never copy
> these patterns into production code.

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

> **Workspace choice:** the canonical `live-search` project uses
> *runtime* queries (`sqlx::query_as::<_, SearchResult>(…)`,
> `sqlx::query(…)`) instead of the `sqlx::query!()` / `sqlx::query_as!()`
> macros shown above. The macros require `cargo sqlx prepare` to maintain
> a `.sqlx/` cache (offline mode) or a live `DATABASE_URL` at compile time
> (online mode). Pick one path per crate and document the choice.

### Pattern 6: Gateway with ServiceModule Trait

```rust
use std::sync::Arc;
use axum::Router;
use futures::future::{BoxFuture, FutureExt};

/// Error returned by service module health checks.
/// String-based reason because the gateway has no opinion about which
/// underlying error type a particular service depends on.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[error("service unavailable: {reason}")]
#[must_use = "a ServiceHealthError must be observed"]
pub struct ServiceHealthError {
    pub reason: String,
}

/// A composable service module mounted under the gateway.
pub trait ServiceModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn path(&self) -> &'static str { self.name() }
    fn description(&self) -> &'static str;
    fn enabled(&self) -> bool { true }
    fn router(&self) -> Router<GatewayState>;

    /// Health check with no arguments — the service knows its dependencies.
    /// Default: always healthy.
    #[must_use = "a health check result should be observed"]
    fn health_check(&self) -> BoxFuture<'_, Result<(), ServiceHealthError>> {
        future::ready(Ok(())).boxed()
    }
}

// Compose all services.
//
// Use `Arc<dyn ServiceModule>` (not `Box<dyn>`) so `GatewayState: Clone`
// stays cheap: cloning the state must not require cloning each registered
// service's heap allocation.
fn build_gateway(state: GatewayState) -> Router {
    let services: Vec<Arc<dyn ServiceModule>> = vec![
        Arc::new(LiveSearchService),
    ];

    let mut router = Router::new()
        .route("/events", get(sse_handler))
        .route("/health", get(health_handler));

    for service in &services {
        if !service.enabled() { continue; }
        // `Router::nest` requires a leading "/" — `/` + path is a tiny
        // allocation but we can avoid it by building the prefix once.
        let prefix = format!("/{}", service.path());
        router = router.nest(&prefix, service.router());
    }

    router
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

> The skill's reference implementation in `references/architecture-patterns.md`
> shows a *simplified* version of this trait. For the real gateway with
> `Jwt`, `Settings`, `LoginRateLimiter`, and aggregated health checks see
> `./gateway/src/gateway.rs` and `./gateway/src/auth.rs`.

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
Fix with a custom handler that probes both paths via
`leptos::server_fn::axum::get_server_fn_service`:

```rust
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;

/// Catch-all handler for server function endpoints.
///
/// Probes the exact path first; if not registered, tries a doubled-prefix
/// variant (e.g. `/api/search` when the `#[server(endpoint = "/api/search")]`
/// macro registered `/api/api/search`).
///
/// # Panics
/// Panics only if the path-rewrite produces an invalid URI — in practice this
/// is infallible because we only ever prepend `/api` to an existing valid URI.
#[expect(
    clippy::expect_used,
    reason = "Path rewrite produces a valid URI by construction (prepending /api to a valid path)"
)]
async fn server_fn_handler(req: Request<Body>) -> impl IntoResponse {
    let method = req.method().clone();
    let original_path = req.uri().path().to_string();
    let (mut parts, body) = req.into_parts();

    let path_to_try =
        if leptos::server_fn::axum::get_server_fn_service(&original_path, method.clone()).is_none()
            && original_path.starts_with("/api/")
        {
            let doubled = format!("/api{original_path}");
            if leptos::server_fn::axum::get_server_fn_service(&doubled, method).is_some() {
                doubled
            } else {
                original_path
            }
        } else {
            original_path
        };

    if path_to_try != parts.uri.path() {
        parts.uri = Uri::try_from(&path_to_try).expect("valid URI from path rewrite");
    }

    let req = Request::from_parts(parts, body);
    leptos_axum::handle_server_fns(req).await
}
```

Mount it with `any` (accepts both GET and POST). For belt-and-braces
compatibility with the Leptos 0.8 macro, register **both** prefixes:

```rust
.route("/api/{*fn_name}",      any(server_fn_handler))
.route("/api/api/{*fn_name}",  any(server_fn_handler))
```

The `server_fn_handler`'s internal probe via
`leptos::server_fn::axum::get_server_fn_service` short-circuits to the
exact registered path, so registering both routes is harmless and avoids
relying on the probe-fallback path alone. This is the form used in
`./live-search/src/main.rs`.

### Pattern 10: SSR + Hydration Setup (Same Crate as Both Bin & Lib)

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "live-search"
path = "src/main.rs"
required-features = ["ssr"]

[features]
ssr = ["dep:leptos_axum", "leptos/ssr"]
hydrate = ["leptos/hydrate"]
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
        // Static WASM + JS bundle — MUST exist for hydration (serve before leptos_routes)
        .nest_service("/pkg", tower_http::services::ServeDir::new("./pkg"))
        // Catch-all server-fn route (see Pattern 9 for the custom handler)
        .route("/api/{*fn_name}", any(server_fn_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(leptos_options.clone())
        // Leptos page routes (returns SSR HTML)
        .leptos_routes(&leptos_options, routes, {
            let opts = leptos_options.clone();
            move || shell(opts.clone())
        })
        .fallback(fallback_handler);

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

> **Note on `axum::serve` signature**: the call `axum::serve(listener, app.into_make_service())`
> shown above is the **stateful** form — `into_make_service()` adapts `Router<S>` into
> `MakeService<S>` so the per-connection `State` extractor works. The canonical
> `live-search/src/main.rs` takes a different (and slightly cheaper) route: it
> converts `Router<LeptosOptions>` to `Router<()>` via `.with_state(...)` and then
> calls `axum::serve(listener, app)` directly, since `LeptosRoutes` handlers
> capture their state via closures rather than via axum's `State` extractor.
> Both forms compile and behave identically at runtime — pick whichever fits
> your routing style. See the `Router<S> → Router<()>` conversion at
> `live-search/src/main.rs:215`.

### Pattern 11: Action.value() vs Action.input()

Both `Action::input()` and `Action::value()` are reactive signals that
**persist for the lifetime of the `Action`** (i.e. for as long as the
component that created it is mounted). They differ in what they hold, not
in whether they survive completion.

> **Important (Leptos 0.8.x):** `Action::value()` returns
> `ArcMappedSignal<Option<O>>` (a reactive signal wrapper), not a plain
> `Option`. Call `.get()` to read its current value, then pattern-match the
> inner `Option<Result<Output, ServerFnError>>`.

- `action.input().get()` → `Option<Input>` — the input that was dispatched
  to the action. Useful for "Showing results for: *&lt;query&gt;*".
- `action.value().get()` → `Option<Result<Output, _>>` — the action's result.
  - `None` while the action is in-flight
  - `Some(Ok(_))` on success
  - `Some(Err(_))` on error
  Useful for "Found N results", "No results found.", error banner.

```rust
// Use value() for post-action result UI (errors, empty results, success).
// Read `.value().get()` ONCE per render frame, then split the inner
// Result — calling .value() twice creates two reactive subscriptions.
let value = move || search_action.value().get();
let results = move || value().and_then(Result::ok);
let error   = move || value().and_then(Result::err);

view! {
    <div id="results">
        {move || match (results(), error()) {
            (None, None) =>
                view! { <p>"Type a query and submit"</p> }.into_any(),
            (_, Some(e)) =>
                view! { <p class="error">{e.to_string()}</p> }.into_any(),
            (Some(items), None) if items.is_empty() =>
                view! { <p>"No results found."</p> }.into_any(),
            (Some(items), None) =>
                view! { <ul>{items}</ul> }.into_any(),
        }}
    </div>
}

// Use input() when you want to echo back what the user submitted.
view! {
    <p>{move || search_action.input().get()
        .map(|q| format!("Showing results for: {q}"))}</p>
}
```

A common mistake is to `match action.value()` directly — that produces a
compile error in Leptos 0.8.x because `value()` returns a signal, not an
`Option`. Always call `.get()` first.

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

    let mut builder = BrowserConfig::builder()
        .user_data_dir(dir.clone())
        .no_sandbox();

    // Allow overriding Chrome binary via CHROME_PATH env var
    if let Ok(chrome_path) = std::env::var("CHROME_PATH") {
        builder = builder.chrome_executable(chrome_path);
    }

    let (browser, mut handler) = Browser::launch(
        builder.build()
    ).await.expect("failed to launch Chromium");

    // Pump CDP events in the background — without this the browser hangs.
    tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await
        .expect("failed to create Chromium page");

    TestContext { browser, page, base_url: base_url() }
}

pub async fn teardown(ctx: TestContext) {
    // Use `eprintln!` (not `tracing::warn!`) for cleanup errors: no
    // `tracing_subscriber::fmt()` is initialised in any E2E test binary, so
    // `tracing::warn!` would be silently dropped on the floor. `eprintln!`
    // always writes to stderr, which the Rust test harness captures per-test
    // and only displays on failure — exactly the right scope for cleanup
    // diagnostics.
    let TestContext { mut browser, page, .. } = ctx;
    if let Err(e) = page.close().await {
        eprintln!("failed to close Chromium page during teardown: {e}");
    }
    if let Err(e) = browser.close().await {
        eprintln!("failed to close Chromium browser during teardown: {e}");
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
// Set via CHROME_PATH env var when running tests
if let Ok(chrome_path) = std::env::var("CHROME_PATH") {
    builder = builder.chrome_executable(chrome_path);
}

// Or inline for debugging:
BrowserConfig::builder()
    .chrome_executable(std::path::PathBuf::from(
        "/path/to/chrome"
    ))
    .build()
```

Use the `CHROME_PATH` environment variable rather than hardcoding paths.
Common locations: Playwright cache (`$PLAYWRIGHT_BROWSERS_PATH/chromium-1208/chrome-linux64/chrome`),
system installation (`/usr/bin/chromium`), or local download.

Verified stable: Chromium **1208** (Playwright 1.50 era).
Crashes observed: Chromium **1223** (Playwright 1.51+ on this host).

### Pattern 14: SSE JSON Injection in Rust Raw Strings

When building SSE event payloads that include JSON, use **raw string literals**
(`r#"..."#`) to avoid escaping JSON braces. For interpolation, prefer explicit
`replace()` over `format!()` when there are many JSON fields — it avoids
confusion between `format!`'s `{field}` placeholders and JSON's `{ }`:

```rust
// Simple case — format! works fine with {{ }} escaping:
let payload = format!(r#"data: {{"query":"{q}","results":[]}}"#);

// For complex JSON payloads, raw string + replace() is more readable:
let payload = r#"data: {"query":"__QUERY__","results":[]}"#
    .replace("__QUERY__", &q);
```

Apply same principle to test JS strings — prefer `replace()` over complex
`format!` with deeply nested `{{ }}` in JS source code.

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
        #[expect(
            clippy::expect_used,
            reason = "signal handler installation can only fail in unrecoverable runtime states"
        )]
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };
        #[cfg(unix)]
        let terminate = async {
            #[expect(
                clippy::expect_used,
                reason = "signal handler installation can only fail in unrecoverable runtime states"
            )]
            let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
            sig.recv().await;
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

#### `biased;` — shutdown wins ties (recommended)

When a notification arrives at the same instant as a cancel signal, `tokio::select!`
picks branches in a non-deterministic order by default. Add `biased;` so the
shutdown branch is **always** checked first:

```rust
loop {
    tokio::select! {
        biased;                     // <-- shutdown wins ties
        _ = shutdown.cancelled() => { break; }
        notification = listener.recv() => { /* ... */ }
    }
}
```

Without `biased;`, the listener may drain one more notification after
shutdown was requested, which can hold the connection open briefly and
produce a non-graceful exit. This is the `async-structured-concurrency`
rule from rust-skills.

> The canonical `live-search/src/db.rs::run_pg_listener` uses `biased;` in
> its inner `select!`. The outer `connect`/`sleep` loop does not, because
> cancellation must always win there too — for which `sleep_or_shutdown`
> is the right idiom.

#### `Send + 'static` for spawned futures

Every `tokio::spawn(...)` requires the future to be `Send + 'static`. That
means:

- Captured data must be `Send` and owned (no `&'a` borrows of stack data).
- `PgNotification`, `axum::response::sse::Event`, and your event types
  must be `Send`. `String` is `Send`; `Rc<T>` is not.
- `serde_json::Value` is `Send` but cannot represent NaN/Inf floats
  safely (a downstream decoder may reject them). Prefer concrete types
  in broadcast payloads.
- `tokio::sync::Mutex` is `Send`; `std::sync::Mutex` is `Send` but **must
  never be held across `.await`** (`async-no-lock-await`).

#### Cancellation safety

Inside `tokio::select!`, a branch that loses the race is dropped. Some
operations are safe to drop; others are not:

| Operation | Cancel-safe? | Notes |
|-----------|--------------|-------|
| `broadcast::Receiver::recv()` | yes | drops the pending read |
| `CancellationToken::cancelled()` | yes | already-future is itself the poll |
| `PgListener::recv()` | yes | sqlx 0.9 drops the TCP read cleanly |
| `tokio::net::TcpListener::accept()` | yes | drops the pending accept |
| `tokio::sync::oneshot::Receiver` | yes | drops the pending receive |
| `tokio::time::sleep` | yes | drops the timer |
| `tokio::io::AsyncReadExt::read_to_end` | **no** | drops the buffer mid-read |
| `tokio::io::AsyncReadExt::read_exact` | **no** | partial read is lost |
| Accumulators (`vec.extend(stream)`) | **no** | partial state lost |

For non-cancel-safe operations, use `tokio::pin!` or move them to a
dedicated task that pushes results into a `mpsc` channel.

### Pattern 16: Newtype IDs for Type-Safe Web Params

Web handlers receive IDs as strings (path params, query params, JSON body
fields). Wrap them in newtypes so the type system prevents mixing a
`UserId` with an `OrgId` (`type-newtype-ids`):

```rust
use std::fmt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash,
         Serialize, Deserialize)]
#[serde(try_from = "Uuid", into = "Uuid")]
#[repr(transparent)]
pub struct UserId(pub Uuid);

impl TryFrom<Uuid> for UserId {
    type Error = UserIdError;
    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        if value.is_nil() { Err(UserIdError::Nil) } else { Ok(Self(value)) }
    }
}

impl From<UserId> for Uuid {
    fn from(value: UserId) -> Uuid { value.0 }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UserIdError {
    #[error("user id must not be the nil UUID")]
    Nil,
}
```

Then in axum handlers:

```rust
async fn get_user(
    Path(user_id): Path<UserId>,    // axum deserializes via TryFrom
) -> Result<Json<User>, AppError> {
    // ...
}
```

This catches at compile time: passing an `OrgId` where a `UserId` is
expected, swapping path param order in a route, etc.

### Pattern 17: Leptos 0.8.x Knowledge Patch

Three Leptos 0.8.x features added after most training cutoffs. Use them
when applicable.

#### `Show` accepts signals directly

Since 0.8.6, `<Show>` accepts the condition as a `Signal`:

```rust
// Pre-0.8.6: wrap in a closure
<Show when=move || user.get().is_some() fallback=|| view! { <Login/> }>

// 0.8.6+: pass the signal directly
<Show when=user is_some fallback=|| view! { <Login/> }>
```

#### `ShowLet` component

`<ShowLet>` is a single-bind shorthand:

```rust
// Equivalent:
<Show when=move || user.get() let=user>
    <p>{move || user.name.to_string()}</p>
</Show>

<ShowLet when=move || user.get() let=user>
    <p>{move || user.name.to_string()}</p>
</ShowLet>
```

#### Bitcode server-function encoding

For binary-heavy server fns, `Bitcode` encoding is faster than the default
JSON. Enable per-fn:

```rust
use leptos::server_fn::codec::Bitcode;

#[server(
    output = Bitcode,
    input = Bitcode,
    endpoint = "/api/large_payload"
)]
pub async fn large_payload() -> Result<Vec<u8>, ServerFnError> {
    // ...
}
```

The client and server must agree on the codec. **No extra `Cargo.toml`
entry is needed** for `bitcode` itself — `leptos::server_fn::codec::Bitcode`
is re-exported by the `server_fn = "0.8"` crate (`pub use bitcode;` in
`server_fn-0.8.13/src/lib.rs`). The codec is therefore available wherever
`leptos` is a dependency. If you want to call `bitcode` APIs directly
outside of `#[server]`, add `bitcode = "0.6"` explicitly.

---

## Common Pitfalls

1. **PgListener connection leak**: Always call `listener.listen()` BEFORE entering the recv loop; the connection is held for the listener's lifetime
2. **Broadcast channel overflow**: Default buffer is 256; lagging consumers should receive an explicit `stream_lagged`/diagnostic event and publishers should log `SendError` when no receivers exist
3. **Leptos SSR hangs**: If a server function never resolves, the SSR stream blocks indefinitely — use `.timeout()` on async operations
4. **JSONB in sqlx macros**: Use `as _` cast for Json<T> in `query!()` macros; otherwise the macro can't infer the type
5. **Feature flag conflicts**: `csr`, `ssr`, `hydrate` are mutually exclusive — use `[features]` section in Cargo.toml to enforce this with `skip_feature_sets`
6. **cross-origin SSE**: EventSource requires same-origin by default; use CORS headers or serve from same domain
7. **chromiumoxide user_data_dir collision**: Default `~/.cache/chromiumoxide-runner/SingletonLock` collides when tests run in parallel. Always set a unique `user_data_dir` per test (see Pattern 12).
8. **WASM hydration requires static serving**: SSR HTML references `/pkg/{crate}.js` and `/pkg/{crate}_bg.wasm`. Without `ServeDir::new("./pkg")` mounted on the router, the page renders but JavaScript never runs. Verify with `curl http://localhost:3000/pkg/live_search.js` returning 200.
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
