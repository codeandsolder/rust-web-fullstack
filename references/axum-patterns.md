# Axum 0.8 Patterns Reference

## Table of Contents
1. [SSE Streaming Endpoint](#1-sse-streaming-endpoint)
2. [Routing & Composition](#2-routing--composition)
3. [State Management](#3-state-management)
4. [Middleware](#4-middleware)
5. [Error Handling](#5-error-handling)
6. [Broadcast → SSE Pattern](#6-broadcast--sse-pattern)
7. [Pitfalls](#7-pitfalls)

---

## 1. SSE Streaming Endpoint

### Basic SSE Handler

```rust
use axum::response::sse::{Event, KeepAlive, Sse};

async fn sse_handler() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = tokio_stream::iter(vec![
        Event::default().data("hello"),
        Event::default().data("world"),
    ]).map(Ok);

    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

Axum auto-sets headers:
- `Content-Type: text/event-stream`
- `Cache-Control: no-cache`

### Event Struct API

```rust
Event::default()
    .data("plain text")                      // sets data: field
    .json_data(serde_json::json!({"k":"v"})) // sets data: as JSON (requires "json" feature)
    .event("custom-event-type")              // sets event: field (for addEventListener)
    .id("unique-123")                        // sets id: field (for Last-Event-ID reconnect)
    .retry(Duration::from_secs(30))          // sets retry: field (milliseconds)
    .comment("ignored by browser")           // sets : field (comment)
```

### KeepAlive Configuration

```rust
KeepAlive::new()
    .interval(Duration::from_secs(15))       // heartbeat interval (default: 15s)
    .text("keep-alive-text")                 // custom comment text
    .event(custom_event)                     // or full custom Event

// Built-in default:
Event::DEFAULT_KEEP_ALIVE  // ":\\n\\n" (empty comment, smallest overhead)
```

KeepAlive resets its timer on each real event. When the inner stream ends, KeepAlive stops. KeepAlive pings only when the stream is idle.

### Dependencies

```toml
[dependencies]
axum = { version = "0.8", features = ["json"] }
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
futures-util = "0.3"
```

---

## 2. Routing & Composition

### Basic Router

```rust
let app = Router::new()
    .route("/", get(index))
    .route("/users", get(list_users).post(create_user))
    .route("/users/{id}", get(show_user).patch(update_user).delete(delete_user));
```

### Path Parameters

```rust
// Single param
async fn show_user(Path(user_id): Path<Uuid>) -> impl IntoResponse { ... }

// Multiple params
async fn team_show(Path((user_id, team_id)): Path<(Uuid, Uuid)>) -> impl IntoResponse { ... }
```

### Query Parameters

```rust
#[derive(Deserialize)]
struct Pagination { page: usize, per_page: usize }

async fn list(Query(p): Query<Pagination>) -> impl IntoResponse { ... }
// Parses ?page=2&per_page=30
```

### Nested Routers

```rust
fn api_routes() -> Router {
    Router::new()
        .route("/posts", get(posts))
        .route("/users", get(users))
}

let app = Router::new()
    .nest("/api", api_routes())  // /api/posts, /api/users
    .nest("/admin", admin_routes());
```

### Merge vs Nest

```rust
// merge: flat addition, no prefix
let app = Router::new()
    .merge(api_router)       // routes from api_router added at current level
    .merge(admin_router);

// nest: prefix addition
let app = Router::new()
    .nest("/api", api_router); // /api/ prefix prepended to all routes
```

### Fallback (404)

```rust
let app = Router::new()
    .route("/users", get(handler))
    .fallback(not_found);  // catch-all for unmatched routes
```

---

## 3. State Management

### Simple State

```rust
#[derive(Clone)]
struct AppState {
    pool: PgPool,
    config: Config,
}

let app = Router::new()
    .route("/", get(handler))
    .with_state(state);

async fn handler(State(state): State<AppState>) -> impl IntoResponse { ... }
```

### Substate via FromRef

```rust
#[derive(Clone, FromRef)]
struct AppState {
    api_state: ApiState,
    db_pool: PgPool,
}

// Handler extracts only what it needs:
async fn api_handler(State(api): State<ApiState>) -> impl IntoResponse { ... }
async fn db_handler(State(pool): State<PgPool>) -> impl IntoResponse { ... }
```

### Broadcast Channel in State

```rust
#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<Event>,
    pool: PgPool,
}

// SSE handler subscribes:
async fn sse(State(state): State<AppState>) -> Sse<...> {
    let rx = state.tx.subscribe();
    Sse::new(BroadcastStream::new(rx).filter_map(|r| async { r.ok() }))
        .keep_alive(KeepAlive::default())
}
```

---

## 4. Middleware

### from_fn Middleware

```rust
use axum::middleware;

async fn auth_middleware(request: Request, next: Next) -> Response {
    // before...
    let response = next.run(request).await;
    // after...
    response
}

let app = Router::new()
    .route("/", get(handler))
    .layer(middleware::from_fn(auth_middleware));
```

### from_fn_with_state (access app state)

```rust
async fn logger(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    tracing::info!("{} {}", request.method(), request.uri());
    next.run(request).await
}

let app = Router::new()
    .route_layer(middleware::from_fn_with_state(state.clone(), logger))
    .with_state(state);
```

### from_extractor Middleware

```rust
#[derive(FromRequestParts)]
struct RequireAuth;

impl<S> FromRequestParts<S> for RequireAuth {
    type Rejection = StatusCode;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        // check auth header
        if parts.headers.get("authorization").is_some() {
            Ok(Self)
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

let app = Router::new()
    .route("/admin", get(admin))
    .route_layer(from_extractor::<RequireAuth>());
```

### Common Middleware (tower-http)

```rust
use tower_http::{
    cors::CorsLayer,
    compression::CompressionLayer,
    trace::TraceLayer,
    limit::RequestBodyLimitLayer,
};

let app = Router::new()
    .layer(CorsLayer::permissive())
    .layer(CompressionLayer::new())
    .layer(TraceLayer::new_for_http());
```

---

## 5. Error Handling

### IntoResponse for Custom Errors

```rust
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("not found")]
    NotFound,
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".into()),
            AppError::Db(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}

// Then handlers can use ?:
async fn handler(pool: State<PgPool>) -> Result<Json<User>, AppError> {
    let user = sqlx::query_as!(User, "SELECT * FROM users WHERE id = $1", id)
        .fetch_one(&*pool).await?;
    Ok(Json(user))
}
```

### Common Error Response Patterns

```rust
// JSON error
(StatusCode::BAD_REQUEST, Json(json!({"error": "bad request"})))

// HTML error
(StatusCode::NOT_FOUND, Html("<h1>Not Found</h1>"))

// Plain text error
(StatusCode::INTERNAL_SERVER_ERROR, "something went wrong".to_string())

// With custom headers
(
    StatusCode::UNAUTHORIZED,
    [(header::WWW_AUTHENTICATE, "Bearer")],
    "unauthorized",
)
```

---

## 6. Broadcast → SSE Pattern

### Complete Example

```rust
use axum::{
    Router,
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use futures_util::StreamExt;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<Event>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (tx, _) = broadcast::channel::<Event>(256);
    let state = AppState { tx };

    let app = Router::new()
        .route("/events", get(sse_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tx.subscribe();

    let stream = BroadcastStream::new(rx)
        .filter_map(|result| async move {
            match result {
                Ok(event) => Some(Ok(event)),
                Err(e) => {
                    tracing::warn!("broadcast lag: {}", e);
                    Some(Ok(Event::default()
                        .event("stream_lagged")
                        .data(e.to_string())))
                }
            }
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// Publisher (background task, HTTP handler, etc.)
async fn publish_update(tx: &broadcast::Sender<Event>, data: &str) {
    let event = Event::default()
        .event("update")
        .data(data.to_string());
    if let Err(e) = tx.send(event) {
        tracing::warn!("no SSE subscribers: {}", e);
    }
}
```

---

## 7. Pitfalls

1. **Sse handler holds broadcast Receiver**: Each SSE connection holds a `broadcast::Receiver`. The channel buffer is shared. If a consumer lags by more than `buffer_size` messages, it's dropped (broadcast semantics).
2. **KeepAlive on ended stream**: If your inner stream ends, the SSE connection closes. KeepAlive only pings while the stream is alive but slow.
3. **State must be Clone**: `with_state()` requires `S: Clone`. PgPool is Clone (Arc internally). broadcast::Sender is Clone.
4. **Route ordering matters**: First matching route wins. Put specific routes before catch-all routes.
5. **Middleware applies to all child routes**: `Router::layer()` applies to all nested routes. Use `route_layer()` for specific routes.
6. **SSE + CORS**: EventSource requires same-origin by default. If serving from a different port in dev, add CORS headers.
7. **Body::from_stream chunking**: Large streams should use `throttle()` or `chunks_timeout()` to avoid overwhelming the network buffer.
8. **BroadcastStream lag**: `BroadcastStream` wraps `broadcast::Receiver`. When the receiver lags (more than `capacity` messages behind), it returns `RecvError::Lagged(n)`. Always handle this gracefully.
