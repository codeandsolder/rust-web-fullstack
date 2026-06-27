# Architecture Patterns Reference

## Table of Contents
1. [Gateway with ServiceModule Trait](#1-gateway-with-servicemodule-trait)
2. [Workspace Structure](#2-workspace-structure)
3. [Live Events via LISTEN/NOTIFY → broadcast → SSE](#3-live-events-via-listennotify--broadcast--sse)
4. [Shared Component Library](#4-shared-component-library)
5. [Database Schemas](#5-database-schemas)
6. [Migration Strategy (from existing projects)](#6-migration-strategy)
7. [Pitfalls](#7-pitfalls)

---

## 1. Gateway with ServiceModule Trait

### Concept

A single axum binary serves multiple Leptos apps (warpproxy, proxytest, searxrs2) under different URL prefixes. Each service implements a `ServiceModule` trait. The gateway composes routers, manages shared state (PgPool, auth, settings), and provides a unified nav shell.

### ServiceModule Trait

```rust
#[async_trait]
pub trait ServiceModule: Send + Sync {
    /// Unique name for this service (used as URL prefix)
    fn name(&self) -> &'static str;

    /// Display name for the shared nav shell
    fn display_name(&self) -> &'static str;

    /// Route prefix (defaults to name)
    fn path(&self) -> &'static str { self.name() }

    /// Build the axum Router for this service
    fn router(&self) -> Router<GatewayState>;

    /// Server function prefix (for leptos_axum::handle_server_fns)
    fn server_fn_path(&self) -> &'static str;

    /// Health check for this service's dependencies
    async fn health_check(&self, pool: &PgPool) -> Result<(), String>;

    /// Whether this service is currently enabled
    fn enabled(&self) -> bool { true }
}
```

### Gateway Composition

```rust
pub fn build_gateway(pool: PgPool) -> Router {
    let (tx, _) = broadcast::channel::<sse::Event>(256);

    let state = GatewayState {
        pool: pool.clone(),
        tx,
        settings: Settings::load().expect("failed to load settings"),
    };

    let services: Vec<Box<dyn ServiceModule>> = vec![
        Box::new(searxrs2::Service),
        Box::new(proxytest::Service),
        Box::new(warpproxy::Service),
    ];

    let mut router = Router::new()
        .route("/events", get(sse_handler))          // unified SSE endpoint
        .route("/health", get(health_handler))        // gateway health
        .nest("/auth", auth_routes());                 // shared auth

    // Compose each service under its path prefix
    for service in &services {
        if !service.enabled() { continue; }
        let service_router = service.router();
        router = router.nest(
            &format!("/{}", service.path()),
            service_router,
        );
    }

    // Shared nav shell (Leptos component rendered server-side)
    router = router.nest("/", shell_routes(services));

    router
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

### GatewayState

```rust
#[derive(Clone, FromRef)]
pub struct GatewayState {
    pub pool: PgPool,
    pub tx: broadcast::Sender<sse::Event>,
    pub settings: Settings,
}
```

### Service Implementation Example

```rust
// searxrs2/src/service.rs
pub struct Service;

#[async_trait]
impl ServiceModule for Service {
    fn name(&self) -> &'static str { "searxrs2" }
    fn display_name(&self) -> &'static str { "Search" }
    fn server_fn_path(&self) -> &'static str { "/searxrs2/api" }

    fn router(&self) -> Router<GatewayState> {
        let routes = generate_route_list(SearxApp);

        Router::new()
            .route("/api/*fn_name", get(handle_server_fns::<SearxApp>))
            .leptos_routes_with_context(
                generate_context(),
                routes,
                {
                    let options = self.leptos_options();
                    move || shell(options.clone())
                },
                None,
            )
            .fallback(file_and_error_handler::<SearxApp>)
    }

    async fn health_check(&self, pool: &PgPool) -> Result<(), String> {
        sqlx::query("SELECT 1").execute(pool).await.map(|_| ()).map_err(|e| e.to_string())
    }
}

fn generate_context() -> impl Fn() + Clone + Send + Sync + 'static {
    let pool = /* from GatewayState */;
    move || {
        provide_context(pool.clone());
    }
}
```

### Why This Pattern

1. **Independently runnable**: Each service can run standalone during development
2. **Shared infrastructure**: Pool, auth, SSE, settings — all shared
3. **URL space collision prevention**: Each service gets its own prefix
4. **Progressive migration**: Services migrate one at a time
5. **Health-aware**: Gateway checks each service's health on startup

---

## 2. Workspace Structure

```
shared/
├── Cargo.toml                  # Workspace root
│
├── corekit/                    # Zero-HTTP utilities
│   └── src/
│       ├── error.rs            # ApiError, chain_dyn
│       ├── auth.rs             # Constant-time eq, bearer extraction
│       ├── log.rs              # tracing_subscriber init
│       └── serde.rs            # Secret-redacted Serialize
│
├── corekit-db/                 # Database init helpers
│   └── src/
│       ├── sqlite.rs           # Connection::open + WAL pragma
│       └── redb.rs             # redb::Builder with cache sizing
│
├── corekit-client/             # wreq client builders
├── corekit-sse/                # SSE transport helpers
├── clickhouse/                 # (deprecated — use PostgreSQL instead)
├── garage/                     # S3/Garage blob storage
│
├── ui/                         # Shared Leptos components
│   └── src/
│       ├── nav.rs              # Shared nav shell
│       ├── auth.rs             # JWT cookie auth state
│       ├── error.rs            # Error boundary + banner
│       └── theme.rs            # Tailwind class constants
│
├── gateway/                    # Service composition
│   └── src/
│       ├── module.rs           # ServiceModule trait
│       ├── router.rs           # Router composition
│       ├── auth.rs             # JWT cookie auth (server-side)
│       └── settings.rs         # Shared settings (SQLite)
│
├── pipeline-core/              # Core types + traits (zero IO)
├── pipeline-fetch/             # HTTP fetcher (wreq)
├── pipeline-parse/             # HTML + JSON parsers
├── pipeline-store/             # Storage (filesystem + PostgreSQL)
├── pipeline-runtime/           # EWMA, P2C scheduling, client pool
└── pipeline/                   # Umbrella re-exports

Each consumer project:
warpproxy/  → depends on: shared, corekit, corekit-db, corekit-client, ui, pipeline
proxytest/  → depends on: shared, corekit, corekit-db, ui, pipeline
searxrs2/   → depends on: shared, corekit, corekit-db, ui, pipeline-runtime
```

### Crate Ownership Rules

- **corekit crates**: zero HTTP deps, every project depends on them
- **ui crate**: shared Leptos components, every frontend depends on it
- **pipeline crates**: fetch-parse-store pipeline, depends on corekit
- **gateway crate**: depends on ui + corekit + all services
- **Consumer projects**: warpproxy, proxytest, searxrs2 — depend on shared crates

---

## 3. Live Events via LISTEN/NOTIFY → broadcast → SSE

### Architecture Diagram

```
┌──────────────┐   writes to    ┌──────────────┐   NOTIFY     ┌──────────────┐
│  Application │ ──────────── │  PostgreSQL    │ ────────── │  PgListener   │
│  (any proc)  │              │  (triggers/    │            │  (gateway)    │
│              │              │   pg_notify)   │            │              │
└──────────────┘              └──────────────┘            └──────┬───────┘
                                                                  │
                                                           broadcast::Sender
                                                                  │
                                                           ┌──────▼───────┐
                                                           │  SSE Handler  │
                                                           │  (axum)       │
                                                           │               │
                                                           │ text/event-   │
                                                           │ stream        │
                                                           └──────┬───────┘
                                                                  │
                                             ┌────────────────────┼────────────────────┐
                                             │                    │                    │
                                      ┌──────▼──────┐    ┌──────▼──────┐    ┌──────▼──────┐
                                      │  Browser #1  │    │  Browser #2  │    │  Browser N  │
                                      │  EventSource  │    │  EventSource  │    │  EventSource  │
                                      │  → Signal     │    │  → Signal     │    │  → Signal     │
                                      └──────────────┘    └──────────────┘    └──────────────┘
```

### Cross-Process Visibility

Multiple processes can observe the same PostgreSQL tables via LISTEN/NOTIFY. The gateway's PgListener subscribes to all channels. When any process writes to a table with a NOTIFY trigger, all SSE clients receive the update.

**Trigger setup** (applied once to PostgreSQL):

```sql
-- Notify on new search results
CREATE OR REPLACE FUNCTION notify_search_results()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('search_results', row_to_json(NEW)::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER on_search_result
AFTER INSERT ON search_results
FOR EACH ROW EXECUTE FUNCTION notify_search_results();

-- Notify on proxy status change
CREATE OR REPLACE FUNCTION notify_proxy_status()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('proxy_status', json_build_object(
        'proxy', NEW.proxy_key,
        'status', NEW.status,
        'latency', NEW.latency
    )::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER on_proxy_status_change
AFTER INSERT OR UPDATE ON proxy_status
FOR EACH ROW EXECUTE FUNCTION notify_proxy_status();
```

### Gateway Listener Task

```rust
async fn run_listener(pool: PgPool, tx: broadcast::Sender<Event>) {
    let mut listener = PgListener::connect_with(&pool).await.unwrap();

    // Subscribe to all relevant channels
    listener.listen_all(vec![
        "search_results",
        "proxy_status",
        "warpproxy_health",
    ]).await.unwrap();

    loop {
        match listener.recv().await {
            Ok(notification) => {
                let event = Event::default()
                    .event(notification.channel().to_string())
                    .data(notification.payload().to_string());
                if let Err(e) = tx.send(event) {
                    tracing::debug!("notification had no SSE receivers: {e}");
                }
            }
            Err(e) => {
                tracing::error!("PgListener error: {}, reconnecting...", e);
                // PgListener auto-reconnects; recv() will work on next call
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}
```

---

## 4. Shared Component Library

### ui/Cargo.toml

```toml
[package]
name = "shared-ui"
version = "0.1.0"
edition = "2024"

[dependencies]
leptos = { version = "0.8", features = ["csr"] }
leptos_router = { version = "0.8", features = ["csr"] }
leptos_meta = { version = "0.8", features = ["csr"] }
gloo-net = { version = "0.6", features = ["eventsource"] }
serde = { version = "1", features = ["derive"] }
shared-corekit = { path = "../corekit" }
```

### Nav Shell Component

```rust
// ui/src/nav.rs
#[component]
pub fn NavShell(
    services: Vec<ServiceNavItem>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <nav class="flex gap-4 p-4 bg-gray-900 text-white">
            <span class="font-bold">"ProxyHub"</span>
            <For
                each=move || services.clone()
                key=|s| s.name.clone()
                children=|s| view! {
                    <a href={s.path} class={move || if s.active { "text-blue-400" } else { "" }}>
                        {s.display_name}
                    </a>
                }
            />
        </nav>
        <main class="p-4">
            {children.map(|c| c())}
        </main>
    }
}

#[derive(Clone)]
pub struct ServiceNavItem {
    pub name: String,
    pub display_name: String,
    pub path: String,
    pub active: bool,
}
```

### Auth State Signal

```rust
// ui/src/auth.rs
use leptos::*;
use gloo_net::eventsource::futures::EventSource;

#[derive(Clone, Debug)]
pub struct AuthState {
    pub user: Option<UserInfo>,
    pub token: Option<String>,
}

pub fn create_auth_state(pool: PgPool) -> AuthState {
    let (auth, set_auth) = signal(AuthState { user: None, token: None });
    provide_context(auth);
    provide_context(set_auth);
    auth.get_untracked()
}

#[component]
pub fn LoginGate(children: Children) -> impl IntoView {
    let auth = use_context::<ReadSignal<AuthState>>().expect("auth state not provided");

    view! {
        <Show
            when=move || auth.get().user.is_some()
            fallback=|| view! { <LoginPage/> }
        >
            {children()}
        </Show>
    }
}
```

---

## 5. Database Schemas

### search_results (shared, used by searxrs2)

```sql
CREATE TABLE search_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    query TEXT NOT NULL,
    engine TEXT NOT NULL,
    data JSONB NOT NULL,
    fts tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(data->>'title', '')), 'A') ||
        setweight(to_tsvector('english', coalesce(data->>'snippet', '')), 'B')
    ) STORED,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_search_fts ON search_results USING GIN(fts);
CREATE INDEX idx_search_created ON search_results (created_at);
```

### proxy_check_results (shared, used by proxytest)

```sql
CREATE TABLE proxy_check_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    proxy_key TEXT NOT NULL,
    proxy_url TEXT NOT NULL,
    target TEXT NOT NULL,
    status_code INT,
    latency_ms FLOAT8,
    success BOOLEAN NOT NULL,
    response_body_hash TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_proxy_check_proxy ON proxy_check_results (proxy_key, created_at);
CREATE INDEX idx_proxy_check_target ON proxy_check_results (target, created_at);
```

### raw_html_storage (shared, used by all)

```sql
CREATE TABLE raw_html_storage (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    blob_hash TEXT NOT NULL UNIQUE,   -- blake3 hash for dedup
    url TEXT NOT NULL,
    content_type TEXT,
    content_length INT,
    body BYTEA,                       -- zstd-compressed
    proxy_used TEXT,
    fetch_duration_ms INT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_raw_html_hash ON raw_html_storage (blob_hash);
CREATE INDEX idx_raw_html_created ON raw_html_storage (created_at);
```

---

## 6. Migration Strategy

### Phase 1: Setup Infrastructure
1. Install PostgreSQL 17
2. Create database
3. Apply initial migrations (shared schema)
4. Set up pg_cron for TTL cleanup
5. Create NOTIFY triggers on shared tables

### Phase 2: Extract Shared Crates
1. Create `shared/` workspace
2. Extract corekit patterns from existing code
3. Extract shared Leptos components into `ui/`
4. Implement `ServiceModule` trait in `gateway/`
5. Verify each crate compiles independently

### Phase 3: Migrate searxrs2 (most mature)
1. Replace SQLite with PostgreSQL
2. Replace SQLite FTS5 with PostgreSQL tsvector
3. Add NOTIFY triggers
4. Test with dual-write (write to both old and new DB)
5. Cut over when verified

### Phase 4: Migrate proxytest
1. Replace ClickHouse with PostgreSQL
2. Map ClickHouse tables to PostgreSQL schemas
3. Add NOTIFY triggers for live updates
4. Wire up to gateway

### Phase 5: Migrate warpproxy
1. Replace SQLite with PostgreSQL
2. Add health check NOTIFY triggers
3. Wire up to gateway

### Phase 6: Gateway Unification
1. Build unified gateway binary
2. Implement shared nav shell
3. Implement JWT auth
4. Implement unified settings (SQLite)
5. Deploy all three services under one domain

---

## 7. Pitfalls

1. **Gateway state contention**: All services share the same PgPool and broadcast channel. Don't let one service's poorly-written queries exhaust the pool.
2. **NOTIFY flood**: High-throughput tables can flood the broadcast channel. Use `NOTIFY` only for user-facing updates, not internal metrics. For high-volume events, batched polling is better.
3. **Circular dependencies**: Leptos UI components depend on corekit types but NOT on database types. If a component needs DB data, receive it via props or signals, not via direct sqlx calls.
4. **Service independence vs sharing**: The gateway pattern allows each service to run standalone. Don't introduce gateway-only dependencies that break standalone mode.
5. **Trigger recursion**: If a trigger updates a table with another trigger, you get infinite recursion. Use `pg_trigger_depth()` guard or `AFTER UPDATE OF specific_column`.
6. **Leptos hot reload**: `cargo leptos watch` works per-project. For gateway development, run the gateway as a regular axum binary (not via cargo-leptos) and use `wasm-pack` for frontend builds.
7. **PostgreSQL connections budget**: 3 projects × ~10 connections + 1 PgListener = 31 connections. Ensure `max_connections >= 50` in postgresql.conf.
