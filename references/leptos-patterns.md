# Leptos 0.8.x Patterns Reference

## Table of Contents
1. [SSR with Axum Integration](#1-ssr-with-axum-integration)
2. [Reactive Primitives](#2-reactive-primitives)
3. [Component Patterns](#3-component-patterns)
4. [Forms & Server Functions](#4-forms--server-functions)
5. [SSE & Streaming](#5-sse--streaming)
6. [Auth Patterns](#6-auth-patterns)
7. [State Management](#7-state-management)
8. [Error Handling](#8-error-handling)
9. [Build System (cargo-leptos)](#9-build-system-cargo-leptos)
10. [Pitfalls & Anti-Patterns](#10-pitfalls--anti-patterns)

---

## 1. SSR with Axum Integration

### Entry Point: render_app_to_stream_with_context

From `leptos/integrations/axum/src/lib.rs:638`:

```rust
pub fn render_app_to_stream_with_context<IV>(
    additional_context: impl Fn() + 'static + Clone + Send + Sync,
    app_fn: impl Fn() -> IV + Clone + Send + Sync + 'static,
) -> impl Fn(Request<Body>) -> Pin<Box<dyn Future<Output = Response<Body>> + Send + 'static>>
```

The SSR flow per request:
1. `Owner::new_root(Some(SsrSharedContext))` creates a fresh reactive tree
2. `provide_contexts()` injects `Parts`, `ResponseOptions`, `RequestUrl`, `ServerMetaContext`
3. `additional_context()` runs custom context setup (DB pool, auth)
4. View rendered via `.to_html_stream_out_of_order()` or `.to_html_stream_in_order()`
5. `<Suspense>` boundaries create `StreamChunk::OutOfOrder` or `StreamChunk::Async` placeholders
6. Response is `PinnedStream<String>` sent as Axum's body via `Body::from_stream()`

### Four SSR Modes

| Mode | TTFB | SEO | JS Required | Use When |
|------|------|-----|-------------|----------|
| `OutOfOrder` (default) | Best | Good | Yes | Dashboards, internal tools |
| `PartiallyBlocked` | Good | Better | Minimal | Mixed public/internal pages |
| `InOrder` | Slowest | Best | Minimal | Public search pages, blog |
| `Async` | Worst | Best | No | Traditional SSR, email templates |

### Shell Function Pattern

```rust
use leptos::*;
use leptos_meta::*;

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options/>
                <MetaTags/>
                <link rel="stylesheet" id="leptos" href="/pkg/myapp.css"/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}
```

---

## 2. Reactive Primitives

### Signal — Fundamental Unit

```rust
// Arena-allocated, Copy, auto-cleaned on owner drop
let (count, set_count) = signal(0);        // (ReadSignal<T>, WriteSignal<T>)
let count = RwSignal::new(0);              // combined read+write

// Arc variants: Clone but not Copy, reference-counted
let (count, set_count) = arc_signal(0);    // (ArcReadSignal<T>, ArcWriteSignal<T>)

// 0.7+ read guards (no clone)
let value = signal.get();       // clones value out
let len = signal.read().len();  // borrows in place (guard)
signal.write().push(42);        // mutable borrow in place
```

### Resource — Async Data Loading

```rust
let todos = Resource::new(
    move || (delete_todo.version().get(), add_todo.version().get()),  // sources
    move |_| get_todos(),  // async fetcher (server function)
);
// Access: todos.get() -> Option<Result<T, E>>
// Use action.version() as source signal to trigger refetch after mutations
```

### Action — Imperative Mutations

```rust
let save_data = ArcAction::new(|task: &String| {
    send_new_todo_to_api(task.clone())
});
save_data.dispatch("My todo".to_string());
// .input() -> Option<I> (current in-flight arg)
// .value() -> Option<O> (last result)
// .pending() -> Memo<bool>
// .version() -> RwSignal<usize> (increments on success)

// For server functions specifically:
let action = ServerAction::<AddTodo>::new();
action.dispatch(form_data);
```

### Memo — Memoized Derivation

```rust
let doubled = Memo::new(move |_| count.get() * 2);
// Only recomputes when .get() actually changes (PartialEq check)
```

### create_slice — Independent Lenses

```rust
let state = RwSignal::new(GlobalState::default());
let (count, set_count) = create_slice(
    state,
    |state| state.count,        // getter
    |state, n| state.count = n, // setter
);
// Setting count only triggers count subscribers, not name subscribers
```

### Owner — Reactive Tree Root

```rust
// Every effect/resource/component gets an Owner
// When dropped, all owned signals/effects are cleaned up
Owner::on_cleanup(|| tracing::info!("component unmounted"));
Owner::pause();   // freeze effects for a subtree
Owner::resume();  // thaw
```

### Cross-Component State Sharing

Three patterns, ordered by preference:

1. **Context API** (shared state): `provide_context(state)`, `use_context::<T>()`
2. **Signal props** (direct): pass `ReadSignal`/`WriteSignal` as component props
3. **Global static** (rarely correct): `static GLOBAL: Lazy<RwSignal<T>>`

```rust
// Provider pattern (from leptos/src/provider.rs:1-49)
#[component]
fn App() -> impl IntoView {
    provide_context(AppState { pool, user: None });
    view! { <Routes/> }
}

#[component]
fn Dashboard() -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState not provided");
    // use state.pool, state.user...
}
```

---

## 3. Component Patterns

### `#[component]` Macro

```rust
#[component]
pub fn SearchBox(
    query: ReadSignal<String>,                      // required
    set_query: WriteSignal<String>,                 // required
    #[prop(optional)] placeholder: Option<String>,   // optional
    #[prop(optional, into)] class: MaybeSignal<String>, // with Into
    children: Children,                              // slot children
) -> impl IntoView { /* ... */ }
```

The macro generates a props struct with `typed_builder` derive. Each function parameter becomes a prop.

### Children Variants

```rust
// TypedChildren — strongly typed slot
fn Panel(children: TypedChildren<Button>) -> impl IntoView { ... }

// Children — type-erased, any view
fn Container(children: Children) -> impl IntoView { ... }

// ViewFnOnce — lazy evaluation (used in Suspense fallback)
fn Suspense(#[prop(optional, into)] fallback: ViewFnOnce) -> impl IntoView { ... }
```

### `#[island]` — Partial Hydration (leptos 0.8.x)

```rust
#[island]
fn InteractiveCounter() -> impl IntoView {
    let (count, set_count) = signal(0);
    view! { <button on:click=move |_| set_count.update(|c| *c += 1)>{count}</button> }
}
// Only this component hydrates on the client; rest is static HTML
```

---

## 4. Forms & Server Functions

### Server Function Definition

```rust
#[server(AddTodo, "/api")]
pub async fn add_todo(title: String) -> Result<Todo, ServerFnError> {
    let pool = use_context::<PgPool>().expect("pool not in context");
    let row = sqlx::query_as!(Todo, "INSERT INTO todos (title) VALUES ($1) RETURNING *", title)
        .fetch_one(&pool).await?;
    Ok(row)
}
```

### ActionForm — Progressive Enhancement

```rust
let add_todo = ServerAction::<AddTodo>::new();

view! {
    <ActionForm action=add_todo>
        <input type="text" name="title" required/>
        <button type="submit">"Add"</button>
    </ActionForm>
    // Use `Display` (`{}`) for user-facing text, not `Debug` (`{:?}`).
    // Add a `impl std::fmt::Display for AddTodoResult` so this compiles.
    <p>{move || add_todo.value().map(|r| format!("Added: {r}"))}</p>
}
```

With JS: form submits via fetch, no page reload. Without JS: form POSTs directly to server function URL (graceful degradation).

### Custom Error Types

Use `thiserror` with `#[from]` to preserve error chains. Do not wrap the
inner error in `String` — that loses the source chain and breaks `?`.

```rust
#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum MyErrors {
    #[error("not found")]
    NotFound,
    /// Preserve the original sqlx error via `#[from]` so `?` works and the
    /// source chain is reachable from `std::error::Error::source()`.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl FromServerFnError for MyErrors {
    type Encoder = JsonEncoding;
    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
        // For generic server-fn errors (network, decode, etc.) we serialize
        // the display string because ServerFnErrorErr isn't a structured type.
        MyErrors::ServerFnError(value.to_string())
    }
}
```

When the inner error is foreign (e.g. `sqlx::Error`, `reqwest::Error`,
`serde_json::Error`), prefer `#[from]` so callers can use `?` and downstream
observability can walk the source chain via `tracing::error!(error = %e, "…")`.

---

## 5. SSE & Streaming

### Server-Side: broadcast → SSE

```rust
use tokio::sync::broadcast;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::wrappers::BroadcastStream;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<Event>,
}

async fn sse_handler(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tx.subscribe();
    Sse::new(BroadcastStream::new(rx).filter_map(|r| async { r.ok() }))
        .keep_alive(KeepAlive::default())
}
```

### Client-Side: EventSource → Signal

EventSource construction can fail (URL parsing, server unreachable) and
`subscribe` can fail (named event may already exist). Surface errors via
`leptos::logging::warn!` or to an `ErrorBoundary` rather than `unwrap` —
this is **client production code**, not a test.

```rust
#[cfg(target_arch = "wasm32")]  // client-only code (WASM only)
{
    let (data, set_data) = signal(String::new());
    let on_error: WriteSignal<Option<String>> = signal(None);

    match EventSource::new("/api/events") {
        Ok(mut es) => match es.subscribe("search_results") {
            Ok(mut stream) => {
                spawn_local(async move {
                    while let Some(msg) = stream.next().await {
                        match msg {
                            Ok(msg) => {
                                if let Some(text) = msg.data().as_string() {
                                    set_data.set(text);
                                } else {
                                    leptos::logging::warn!("SSE message had non-string data");
                                }
                            }
                            Err(e) => leptos::logging::error!("SSE stream error: {e:?}"),
                        }
                    }
                });
                on_cleanup(move || es.close());
            }
            Err(e) => {
                leptos::logging::error!("failed to subscribe to SSE: {e:?}");
                on_error.set(Some(format!("subscribe failed: {e}")));
            }
        },
        Err(e) => {
            leptos::logging::error!("failed to open SSE connection: {e:?}");
            on_error.set(Some(format!("connection failed: {e}")));
        }
    }

    // Render the error banner if either EventSource::new or subscribe failed
    Effect::new(move |_| {
        if let Some(msg) = on_error.get() {
            // surface to <ErrorBoundary> or visible banner
        }
    });
}
```

For network retry, wrap with `gloo_timers::future::TimeoutFuture` and
backoff on `onerror`. Liveness reconnection is built into the browser
`EventSource` (it reconnects automatically after 3 s default).

### StreamingText — Server-Sent Strings

```rust
#[server(output = StreamingText)]
pub async fn file_progress(filename: String) -> Result<TextStream, ServerFnError> {
    let progress = progress::for_file(&filename);
    let progress = progress.map(|bytes| Ok(format!("{bytes}\n")));
    Ok(TextStream::new(progress))
}
```

### ReadSignal from Stream

```rust
// Convert any stream to a ReadSignal
let s = ReadSignal::from_stream_unsync(my_stream);
let s = ReadSignal::<T>::from_stream(my_stream);  // requires Send
```

### Suspense + Streaming SSR

```rust
view! {
    <Suspense fallback=|| view! { <p>"Loading search results..."</p> }>
        {move || Suspend::new(async move {
            let results = resource.await;
            view! { <SearchResultsList data=results/> }
        })}
    </Suspense>
}
```

During OutOfOrder SSR, Suspense sends the fallback immediately, then a `<script>` replaces it when data resolves. This is how Leptos achieves streaming SSR with good TTFB.

---

## 6. Auth Patterns

### Session Auth with Axum

Based on `projects/session_auth_axum` example:

```rust
let session_config = SessionConfig::default().with_table_name("axum_sessions");
let auth_config = AuthConfig::<i64>::default();
let session_store = SessionStore::<SessionSqlitePool>::new(
    Some(SessionSqlitePool::from(pool.clone())), session_config,
).await.unwrap();

let app = Router::new()
    .leptos_routes(&app_state, routes, move || shell(options.clone()))
    .layer(AuthSessionLayer::<User, i64, SessionSqlitePool, SqlitePool>::new(...))
    .layer(SessionLayer::new(session_store))
    .with_state(app_state);
```

### Server Function Auth Gate

```rust
#[server(GetUserData)]
pub async fn get_user_data() -> Result<UserData, ServerFnError> {
    let auth = auth().await?;  // extract AuthSession from context
    let user = auth.current_user.ok_or(ServerFnError::new("not logged in"))?;
    let data = sqlx::query_as!(UserData, "SELECT * FROM user_data WHERE user_id = $1", user.id)
        .fetch_one(&pool).await?;
    Ok(data)
}
```

### Redirect After Login

```rust
#[server(Login, "/api")]
pub async fn login(username: String, password: String) -> Result<(), ServerFnError> {
    // verify credentials...
    auth.login_user(user.id);
    leptos_axum::redirect("/dashboard");
    Ok(())
}
```

### Cookie Reading (Manual)

```rust
// In a server function:
let parts = use_context::<Parts>().expect("request parts not provided");
let cookie_header = parts.headers.get("cookie");

// Set cookie via ResponseOptions:
let res = expect_context::<ResponseOptions>();
res.insert_header(
    header::SET_COOKIE,
    HeaderValue::from_str("session=abc123; Path=/; HttpOnly").unwrap(),
);
```

---

## 7. State Management

### Context Hierarchy

Context travels UP the Owner tree. If you `provide_context` in a child component, the parent cannot see it. For shared state visible to all children, provide at the root:

```rust
#[component]
fn App() -> impl IntoView {
    let app_state = AppState { pool: create_pool(), config: load_config() };
    provide_context(app_state);
    view! {
        <Router>
            <Routes/>
        </Router>
    }
}
```

### ResponseOptions (SSR)

```rust
// Get from context during SSR:
let res = expect_context::<ResponseOptions>();
res.set_status(StatusCode::NOT_FOUND);
res.insert_header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
```

### URL State

```rust
// leptos_router provides URL info via context:
let params = use_params_map();
let query = use_query_map();
let location = use_location();

// Navigate programmatically:
let navigate = use_navigate();
navigate("/search?q=rust", NavigateOptions::default());
```

---

## 8. Error Handling

### ErrorBoundary Component

```rust
view! {
    <ErrorBoundary fallback=|errors| {
        view! {
            <div class="error-banner">
                <h2>"Something went wrong"</h2>
                <For each=move || errors.get().into_iter()
                    key=|(_, e)| e.to_string()
                    children=|(_, e)| view! { <p>{e.to_string()}</p> }
                />
            </div>
        }
    }>
        <MainContent/>
    </ErrorBoundary>
}
```

### Catching Errors from Server Functions

```rust
let result: Result<Data, ServerFnError> = get_data().await;
match result {
    Ok(data) => view! { <DataView data/> },
    Err(e) => view! { <ErrorDisplay error=e.to_string()/> },
}
```

### Component-Level Error Handling

```rust
#[component]
fn FallibleComponent() -> impl IntoView {
    let data = Resource::new(|| (), |_| fallible_api_call());
    view! {
        <Suspense fallback=|| view! { <Loading/> }>
            {move || match data.get() {
                Some(Ok(d)) => view! { <DataView data=d/> }.into_any(),
                Some(Err(e)) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                None => view! {}.into_any(),
            }}
        </Suspense>
    }
}
```

---

## 9. Build System (cargo-leptos)

### Configuration

```toml
# Cargo.toml
[package.metadata.leptos]
output-name = "myapp"
site-root = "target/site"
site-pkg-dir = "pkg"
tailwind-input-file = "input.css"
assets-dir = "public"
site-addr = "127.0.0.1:3000"
reload-port = 3001
bin-features = ["ssr"]
bin-default-features = false
lib-features = ["hydrate"]
lib-default-features = false

[package.metadata.leptos.tailwind]
input-file = "style/input.css"
config-file = "tailwind.config.js"

[features]
ssr = ["leptos/ssr", "leptos_axum"]
hydrate = ["leptos/hydrate"]
```

### Feature Flag Rules

Feature flags are **mutually exclusive** per build target:
- Server binary: `features = ["ssr"]`
- Client WASM: `features = ["hydrate"]` (SSR hydration) or `features = ["csr"]` (pure client)
- `csr + ssr`, `csr + hydrate`, `ssr + hydrate` are **forbidden**

### Dual Crate Type

```toml
[lib]
crate-type = ["cdylib", "rlib"]
```

`cdylib` for WASM output, `rlib` for server binary. Both from the same lib crate.

### Commands

```bash
cargo leptos watch      # dev server with hot reload
cargo leptos build      # production build
cargo leptos end-to-end # build + run playwright tests
```

---

## 10. Pitfalls & Anti-Patterns

1. **SSR context not provided**: Server functions called outside `render_app_to_stream_with_context` have no `Parts`, `PgPool`, etc. — they'll panic.
2. **Signal in SSR**: `signal()` works in SSR but doesn't survive across requests. Each request gets a fresh reactive tree.
3. **EventSource on SSR**: `gloo_net::EventSource` panics on server. Always wrap in `#[cfg(not(feature = "ssr"))]`.
4. **on:click inside SSR**: Event listeners are no-ops during SSR. They only activate during hydration or in CSR.
5. **Large resources blocking SSR**: Unwrapped `Resource::get()` in synchronous view returns `None` during SSR (because it needs to suspend). Use `<Suspense>` or `Suspend::new()`.
6. **Tailwind purge in dev**: `cargo-leptos` runs Tailwind JIT in watch mode. CSS changes are hot-reloaded.
7. **WebSocket vs SSE**: Leptos has no built-in WebSocket support. Use SSE for server-to-client, server functions for client-to-server.
8. **Hydration mismatch**: If SSR HTML and CSR render produce different DOM, hydration fails silently. Fix by ensuring deterministic rendering or using `<Suspense>` for non-deterministic content.
