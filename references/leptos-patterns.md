# Leptos 0.8 Patterns Reference

Companion to [`SKILL.md`](../SKILL.md). Examples here match the current
`live-search` and `i18n-demo` crates.

## SSR and hydration builds

A full-stack Leptos crate is built for two targets:

- native target with the crate's `ssr` feature,
- `wasm32-unknown-unknown` with the crate's `hydrate` feature.

Keep server-only dependencies behind native/SSR gates and browser-only code
behind `wasm32`/hydrate gates. `cargo check --workspace --all-targets` will also
compile combinations where application features are absent, so server-function
bodies that refer to SSR-only code need appropriate `#[cfg(feature = "ssr")]`
gating.

## Server functions

`endpoint` is relative to Leptos's server-function prefix. With the default
`/api` prefix:

```rust
#[server(endpoint = "search")]
pub async fn search(query: String) -> Result<..., ServerFnError> { ... }
```

is reached at `/api/search`.

Canonical Axum catch-all:

```rust
.route("/api/{*fn_name}", any(leptos_axum::handle_server_fns))
```

Do not add `/api/api/*` and do not write `endpoint = "/api/search"` to compensate
for it.

Normal server-function encodings are subject to the server framework's request
body limits. If an application intentionally sends large request bodies, raise
Axum's limit explicitly for the relevant route rather than assuming the macro
bypasses it.

## Action state

`Action::value()` is the most recently completed result. It is not an in-flight
flag. Once an action has completed at least once, a later dispatch can leave that
old value present while the new request is pending.

Use:

```rust
let pending = move || action.pending().get();
let latest = move || action.value().get();
```

The live-search UI uses `pending()` for loading state, de-duplicates the debounce
and explicit-submit paths, and suppresses old results when the query is cleared.

## Search input validation

If an error says “characters,” validate characters rather than UTF-8 bytes:

```rust
let len = query.trim().chars().count();
```

Do not send raw SQL/database errors back through `ServerFnError`; log them with
`tracing` and return a stable client-facing error.

## Router and TraceLayer

Build all application/SSR routes before calling `Router::layer` for middleware
that must cover the whole app:

```rust
let router = Router::new()
    .route("/api/{*fn_name}", any(handle_server_fns))
    .leptos_routes(&options, routes, shell)
    .route("/health", get(health))
    .fallback(not_found)
    .layer(TraceLayer::new_for_http());
```

Routes added after an earlier `Router::layer()` do not retroactively inherit it.

## Hydration assets

SSR without JS/WASM assets produces a page that looks correct but does not
hydrate. The application shell and static server must agree about `/pkg/*`.

Browser E2E should use real built assets and the real Leptos route tree.

## Stylance

The Stylance macro and CLI do different jobs:

- Rust macro: exposes generated/hashed class identifiers to Rust code,
- `stylance build`: transforms/bundles the CSS artifact.

A production build that uses Stylance needs the CLI build step. Do not describe
CSS bundling as happening automatically just because the Rust proc macro ran.

## SSE client

Named SSE events are independent subscriptions. If the server emits:

- `connected`,
- `search_result`,
- `stream_lagged`,

the client must subscribe to every name it intends to handle.

Do not set a Connected UI state immediately after `EventSource::new`; that only
means the browser object was constructed. The canonical live-search client waits
for the named server `connected` event.

## Search results and pending requests

Because asynchronous requests can overlap, a production search UI should avoid
letting an older response overwrite a newer query. Keyed `Resource`s, request
sequence numbers, or cancellation are all viable approaches depending on the
application. The current demo de-duplicates identical dispatches; if its request
model becomes more complex, add explicit stale-response protection.

## ErrorBoundary

Use `ErrorBoundary` for rendering errors/recoverable component failures, not as a
substitute for normal `Result` handling from server functions. Server failures
should be represented explicitly in UI state.

## i18n

`leptos_i18n` generates compile-time-checked locale keys and `Locale` values.
Use dedicated keys for the UI concept being translated instead of recycling an
unrelated key.

The i18n demo SSR shell starts in English and the hydrated client updates the
root `<html lang>` attribute when switching EN/DE. Locale persistence and
Accept-Language negotiation are intentionally outside this minimal example.

## Islands

On the pinned Leptos 0.8 line, the relevant Cargo feature is `islands`.
Do not document `experimental-islands`.

## Scoped styles vs inline styles

Inline `<style>` blocks are convenient for tiny demos but require a CSP that
permits inline style. For a production strict CSP, serve stylesheet assets or
use nonce/hash-based policy deliberately.

## Feature-matrix checks

When adding an optional feature, check it in CI explicitly. At minimum this
workspace checks:

- native workspace/default targets,
- `live-search` SSR,
- `live-search` SSR + OTel,
- `live-search` hydrate/WASM,
- `i18n-demo` SSR,
- `i18n-demo` hydrate/WASM.

A feature that is never compiled in CI is not a maintained example.
