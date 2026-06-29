# Architecture Patterns Reference

## Table of Contents
1. [Workspace Structure](#1-workspace-structure)
2. [Gateway with ServiceModule Trait](#2-gateway-with-servicemodule-trait)
3. [Live Events via LISTEN/NOTIFY → broadcast → SSE](#3-live-events-via-listennotify--broadcast--sse)
4. [Database Schema (live-search)](#4-database-schema-live-search)
5. [SSR + WASM Hydration](#5-ssr--wasm-hydration)
6. [Testing Architecture](#6-testing-architecture)
7. [Pitfalls](#7-pitfalls)

---

## 1. Workspace Structure

```
rust-web-fullstack/
├── Cargo.toml                  # Workspace root (edition 2024)
├── rust-toolchain.toml         # Pinned Rust 1.94.0 + WASM target
├── Makefile                    # Developer workflow
├── docker-compose.yml          # Postgres + live-search + gateway
├── .woodpecker.yml             # CI pipeline (check, clippy, test, e2e)
│
├── gateway/                    # Pure axum + JWT + service registry
│   ├── Cargo.toml              # Depends on: axum, jsonwebtoken, sqlx, tokio
│   └── src/
│       ├── main.rs             # Bin entry point (service composition)
│       ├── module.rs           # ServiceModule trait definition
│       ├── gateway.rs          # Gateway state + router composition
│       └── auth.rs             # JWT cookie auth
│
├── live-search/                # Leptos SSR + WASM hydrate + sqlx + SSE
│   ├── Cargo.toml              # Dual cdylib+rlib, feature gates ssr/hydrate
│   ├── migrations/             # sqlx migrations
│   └── src/
│       ├── main.rs             # Server binary (ssr feature)
│       ├── lib.rs              # Library root (shared between bin + WASM)
│       ├── app.rs              # Leptos App component + shell
│       ├── db.rs               # PgPool + PgListener (LISTEN/NOTIFY)
│       ├── sse.rs              # SSE handler
│       └── events.rs           # SseEvent types
│
├── e2e-tests/                  # chromiumoxide-based browser E2E
│   ├── Cargo.toml
│   ├── tests/
│   │   ├── common.rs           # Shared helpers (unique_profile_dir, wait_for_js_true)
│   │   ├── live_search_test.rs # Leptos SSR + search form tests
│   │   ├── sse_test.rs         # SSE live update tests
│   │   └── gateway_test.rs     # Gateway health + auth tests
│   └── screenshots/            # Baseline screenshots for diff testing
│
├── gateway.Dockerfile          # Multi-stage build (gateway only)
├── live-search.Dockerfile      # Multi-stage build (WASM + SSR)
│
└── scripts/
    ├── init-db.sql             # Docker entrypoint init (extensions only)
    ├── seed-db.sh              # Idempotent seed data
    └── test-e2e.sh             # Local E2E test runner
```

### Crate Roles

| Crate | Type | Purpose |
|-------|------|---------|
| `live-search` | Binary + Library | Main application: Leptos SSR frontend + sqlx backend + SSE live updates |
| `gateway-example` | Binary | API gateway: routes requests, JWT auth, service registry |
| `e2e-tests` | Test-only | chromiumoxide + reqwest browser-driven E2E tests |

### Feature Flag Discipline

`live-search` uses mutually exclusive features per build target:

```toml
[features]
ssr = ["dep:leptos_axum", "leptos/ssr"]
hydrate = ["leptos/hydrate"]
```

- Server binary: `--features ssr` (compiles main.rs, axum, sqlx, etc.)
- WASM library: `--features hydrate` (compiles lib.rs without server code)
- `csr + ssr`, `csr + hydrate`, `ssr + hydrate` are **forbidden**

---

## 2. Gateway with ServiceModule Trait

### ServiceModule Trait

```rust
use axum::Router;
use futures::future::{BoxFuture, FutureExt};

/// Error returned by service module health checks.
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

    #[must_use = "a health check result should be observed"]
    fn health_check(&self) -> BoxFuture<'_, Result<(), ServiceHealthError>> {
        future::ready(Ok(())).boxed()
    }
}
```

Key design decisions:
- `health_check` takes no arguments — the service knows how to reach its dependencies
- `ServiceHealthError` is `#[non_exhaustive]` + `#[must_use]` + string-based reason
- The trait is object-safe (no `async_trait`), storing `Box<dyn ServiceModule>`

### Gateway Composition

```rust
pub fn build_gateway(pool: PgPool, tx: broadcast::Sender<SseEvent>) -> Router {
    let state = GatewayState { pool: pool.clone(), tx };

    let services: Vec<Box<dyn ServiceModule>> = vec![
        Box::new(live_search::Service),
    ];

    let mut router = Router::new()
        .route("/events", get(sse_handler))
        .route("/health", get(health_handler));

    for service in &services {
        if !service.enabled() { continue; }
        router = router.nest(
            &format!("/{}", service.path()),
            service.router(),
        );
    }

    router
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

### GatewayState

```rust
#[derive(Clone)]
pub struct GatewayState {
    pub pool: PgPool,
    pub tx: broadcast::Sender<SseEvent>,
}
```

---

## 3. Live Events via LISTEN/NOTIFY → broadcast → SSE

### Architecture Diagram

```
┌──────────────┐   NOTIFY      ┌──────────────┐  broadcast   ┌──────────────┐
│  PostgreSQL   │ ──────────── │  PgListener   │ ──────────── │  SSE Handler  │
│  (trigger)    │              │  (live-search) │              │  (axum)       │
└──────────────┘              └──────┬───────┘              └──────┬───────┘
                                      │                             │
                               tokio::broadcast              text/event-stream
                                      │                             │
                                ┌─────▼─────┐              ┌───────▼───────┐
                                │  PgListener │              │  Leptos Client │
                                │  Task       │              │  EventSource   │
                                └───────────┘              └───────────────┘
```

### PgListener Task

```rust
async fn run_pg_listener(
    pool: PgPool,
    tx: broadcast::Sender<SseEvent>,
    shutdown: CancellationToken,
) {
    let mut listener = PgListener::connect_with(&pool).await
        .expect("failed to connect PostgreSQL listener");

    listener.listen("search_results").await
        .expect("failed to subscribe to search_results channel");

    loop {
        tokio::select! {
            notification = listener.recv() => {
                match notification {
                    Ok(n) => {
                        let event = SseEvent { channel: n.channel().to_string(), payload: n.payload().to_string() };
                        if let Err(e) = tx.send(event) {
                            tracing::debug!("notification had no SSE receivers: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::error!("PgListener error: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
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

### NOTIFY Trigger

```sql
CREATE OR REPLACE FUNCTION notify_search_result()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('search_results', row_to_json(NEW)::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER on_search_result_insert
    AFTER INSERT ON search_results
    FOR EACH ROW EXECUTE FUNCTION notify_search_result();
```

---

## 4. Database Schema (live-search)

### search_results

```sql
CREATE TABLE search_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    snippet TEXT NOT NULL,
    fts tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(snippet, '')), 'B')
    ) STORED,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_search_fts ON search_results USING GIN(fts);
```

### Connection Budgeting

Single binary with PgListener:

```sql
max_connections = 20          -- PostgreSQL config (512MB baseline)
pool.max_connections = 19     -- for queries
                              -- 1 reserved for PgListener
```

For multi-process deployments, divide connections across processes:

```sql
max_connections = 20          -- PostgreSQL config
-- Each process: max_connections = (20 - 3_superuser) / N_processes
```

---

## 5. SSR + WASM Hydration

### Build Pipeline

```
cargo build --release -p live-search --lib --target wasm32-unknown-unknown --features hydrate
wasm-bindgen --target web --out-dir pkg --out-name live_search target/wasm32-unknown-unknown/release/live_search.wasm
cargo build --release -p live-search --features ssr
```

### Router Setup

```rust
// Axum router with SSR + SSE + static files
let app = Router::new()
    .nest_service("/pkg", ServeDir::new("./pkg"))
    .route("/api/events", get(sse::sse_handler))
    .route("/api/{*fn_name}", any(server_fn_handler))  // see Pattern 9
    .layer(TraceLayer::new_for_http())
    .with_state(leptos_options.clone())
    .leptos_routes(&leptos_options, routes, {
        let lo = leptos_options.clone();
        move || app::shell(lo.clone())
    })
    .fallback(fallback_handler);
```

### Server Function Path Handling

Leptos 0.8's `#[server(endpoint = "/api/search")]` combined with `handle_server_fns` at `/api/{*fn_name}` can register the function at `/api/api/search`. The live-search binary uses a custom handler that checks both paths:

```rust
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

---

## 6. Testing Architecture

### Layer Map

| Layer | Tool | Scope |
|-------|------|-------|
| Unit | `#[cfg(test)]` in-source | Individual functions, sqlx query tests |
| Integration | `tests/` in `e2e-tests/` | Service-level tests with real DB |
| E2E | chromiumoxide + reqwest | Browser-driven tests against running services |
| Visual | Chrome DevTools MCP | Ad-hoc exploration (not CI) |

### chromiumoxide Setup

Every test MUST:
1. Use a unique `user_data_dir` per test (pattern with process ID + nanos + counter)
2. Pump the CDP handler in a background task
3. Assert failures visibly (no silent skips)

### CI Pipeline

```yaml
# .woodpecker.yml — three stages:
# 1. check-workspace: cargo check all targets
# 2. unit-tests: cargo test --lib
# 3. clippy: cargo clippy -D warnings
# 4. fmt: cargo fmt --check
# 5. e2e-tests: build WASM + SSR, start services, run chromiumoxide tests
```

---

## 7. Pitfalls

1. **Server fn path doubling**: `endpoint = "/api/search"` + `handle_server_fns` at `/api/{*fn_name}` → function reachable at `/api/api/search`. Fix with the custom handler above.
2. **PgListener connection leak**: PgListener holds 1 pool connection for its lifetime. Budget accordingly.
3. **Pool contention**: All queries + PgListener share the same pool. If the pool is exhausted, queries block.
4. **PostgreSQL connections budget**: single binary needs `max_connections >= 20` for production workloads (19 for queries + 1 for PgListener). For multi-process, multiply by process count.
5. **WASM hydration requires static serving**: SSR HTML references `/pkg/*`. Without `ServeDir::new("./pkg")`, the page renders but JavaScript never runs.
6. **Feature flag conflicts**: `ssr`, `hydrate`, `csr` are mutually exclusive per build target. Use `[features]` in Cargo.toml with separate build commands.
7. **chromiumoxide user_data_dir collision**: Default `~/.cache/chromiumoxide-runner/SingletonLock` collides in parallel tests. Always set unique per test.
8. **Broadcast channel overflow**: Default buffer is 256. If SSE consumers lag, they get `RecvError::Lagged`. Handle gracefully with diagnostics.
