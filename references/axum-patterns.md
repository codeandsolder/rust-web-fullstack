# Axum 0.8 Patterns Reference

This is a compact companion to [`SKILL.md`](../SKILL.md). The repository code is
the source of truth.

## Router construction

Basic composition:

```rust
let app = Router::new()
    .route("/users", get(list_users).post(create_user))
    .nest("/api", api_routes())
    .fallback(not_found);
```

`merge` adds another router at the current level; `nest` adds a path prefix.
Axum resolves routes by its routing table, not by a generic “first matching route
wins” rule. Do not rely on textual route declaration order to disambiguate routes.

### Global middleware placement

`Router::layer(layer)` applies that layer to routes that already exist on the
router at the time it is called. If routes are added afterward (including
Leptos-generated routes), they do not retroactively inherit the earlier layer.

Canonical shape:

```rust
let app = Router::new()
    .route("/api/{*fn_name}", any(handle_server_fns))
    .leptos_routes(&options, routes, shell)
    .route("/health", get(health))
    .fallback(not_found)
    .layer(TraceLayer::new_for_http());
```

Use `route_layer` for middleware that should apply only to routes already present
in a sub-router or route method.

## State

Prefer clone-cheap state handles:

```rust
#[derive(Clone)]
struct AppState {
    pool: PgPool,
    tx: broadcast::Sender<MyEvent>,
    config: Arc<Config>,
}

async fn handler(State(state): State<AppState>) { /* ... */ }
```

`PgPool` and `broadcast::Sender` are cheap to clone internally.

## SSE

Axum's SSE response handles the core SSE headers. Use named events deliberately:

```rust
let event = Event::default()
    .event("search_result")
    .json_data(payload)?;
```

A browser using `addEventListener("search_result", ...)` or an EventSource helper
subscribed to that name will not receive a different named event such as
`connected` or `stream_lagged` through that subscription.

Broadcast lag is not connection termination. `tokio::sync::broadcast` drops old
messages for a lagging receiver and reports `RecvError::Lagged(n)`; handle it
explicitly and decide whether the client needs a resync signal.

`KeepAlive` only keeps an otherwise-live stream active. If the inner stream ends,
the SSE response ends.

## Error handling

Internal errors should be logged internally and mapped to stable client-safe
responses:

```rust
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("not found")]
    NotFound,
    #[error("database error")]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "not found"})),
            ).into_response(),
            Self::Db(error) => {
                tracing::error!(error = %error, "database request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal error"})),
                ).into_response()
            }
        }
    }
}
```

Do **not** return `sqlx::Error::to_string()` or other implementation details to
clients merely because the handler already has the error available.

Upstream service failures should normally map to gateway-oriented statuses such
as `502 Bad Gateway` or `503 Service Unavailable`, depending on the failure model.

## Request bodies

Axum has request-body limits. Leptos server functions using normal encodings are
also subject to the framework's body limit. If a route intentionally accepts a
large body, configure `DefaultBodyLimit`/the appropriate limit explicitly and
keep per-route limits as narrow as practical.

## CORS with credentials

Credentialed CORS cannot be combined with wildcard origin/method/header policies.
For cookie sessions, enumerate the allowed origins and the required methods and
headers, then enable credentials.

Do not infer a trusted client address from `X-Forwarded-For` until a trusted
reverse proxy boundary strips untrusted forwarding headers.

## Sessions and CSRF

The gateway deliberately demonstrates cookie sessions alongside Bearer auth.
Session middleware must run before middleware that extracts `Session` (including
the CSRF layer). Mutating session routes are CSRF protected; bootstrap/login and
refresh routes use their own credential models and are not synchronizer-token
protected.

Session backend failures are server errors. Do not turn a failed session read or
flush into HTTP 200 with an `{ "error": ... }` body.

## CSP

A custom `from_fn` CSP middleware is fine, but it is not required because Axum's
response body lacks `Clone`; `SetResponseHeaderLayer` does not impose that body
bound. If custom middleware is retained, document the actual reason (for example,
policy construction or if-not-present behavior) rather than a nonexistent Tower
restriction.

## WebSockets

For browser-facing WebSockets:

- validate `Origin` against the expected host/allowlist,
- set `WebSocketUpgrade::max_message_size` and `max_frame_size` before upgrade,
- handle slow-client/backpressure behavior,
- connect active sockets to application shutdown in production services.

The `i18n-demo` WebSocket is intentionally in-memory but demonstrates the first
three points.

## Observability

`TraceLayer` creates request tracing, while OpenTelemetry propagation additionally
requires middleware that extracts incoming `traceparent`/`tracestate`. Setting a
global W3C propagator alone does not read HTTP headers.
