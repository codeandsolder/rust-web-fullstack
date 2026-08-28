---
name: rust-web-fullstack
description: Full-stack Rust web development with Leptos 0.8, Axum 0.8, PostgreSQL/sqlx 0.9, SSR/hydration, SSE/LISTEN-NOTIFY, JWT/session auth, testcontainers, and chromiumoxide E2E testing. Use for architecture, implementation, review, debugging, and deployment of this stack.
---

# Rust Web Fullstack — Canonical Guide

This file describes the patterns that are actually implemented in this repository.
When prose and code disagree, treat the code plus its tests as authoritative and
fix this document in the same change.

## Workspace map

| Path | Purpose |
|---|---|
| `live-search/` | Leptos SSR/hydration, PostgreSQL search, cache, SSE, PgListener |
| `gateway/` | Axum gateway, JWT + rotating refresh tokens, sessions/CSRF, OpenAPI |
| `i18n-demo/` | `leptos_i18n` locale switching and a small WebSocket example |
| `crates/config/` | typed TOML + `RWF_*` config and validation |
| `crates/domain/` | framework-free domain types and invariants |
| `e2e-tests/` | testcontainers HTTP integration and chromiumoxide browser E2E |
| `migrations/` | the single SQLx migration history for the shared database |
| `.github/workflows/ci.yml` | format/check/clippy/build/Docker/browser CI |

The workspace pins Rust 1.94 as its supported toolchain/MSRV. Newer stable Rust
versions do not by themselves justify raising that floor.

## Build and test

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib

rustup target add wasm32-unknown-unknown
cargo leptos build --release -p live-search
cargo leptos build --release -p i18n-demo
```

Browser E2E additionally needs Docker, Chromium, and built Leptos assets:

```bash
LIVE_SEARCH_PKG_DIR=live-search/target/site/pkg \
CHROME_PATH=/usr/bin/chromium \
cargo test --release --locked -p e2e-tests --tests \
  --features browser-tests -- --test-threads=1 --nocapture
```

The GitHub Actions workflow is the executable reference for CI prerequisites.

## Critical rules

### 1. Keep SSR and hydration feature sets target-specific

A Leptos crate is compiled twice: native SSR and `wasm32-unknown-unknown`
hydration. Do not enable both application modes indiscriminately in one target.
The `package.metadata.leptos` sections in the example crates show the intended
split.

### 2. `#[server(endpoint = ...)]` endpoints are relative to the server-fn prefix

With Leptos's default `/api` prefix:

```rust
#[server(endpoint = "search")]
async fn search(...) { ... }
```

is served under `/api/search`. Do **not** write `endpoint = "/api/search"` and
do not add an `/api/api/*` compatibility route. The old doubled-prefix pattern
was a configuration bug, not a Leptos requirement.

The canonical catch-all is:

```rust
.route("/api/{*fn_name}", any(leptos_axum::handle_server_fns))
```

Normal server-function encodings are subject to the web framework's request-body
limit. If an endpoint intentionally accepts a large non-multipart body, configure
Axum's `DefaultBodyLimit` deliberately rather than assuming server functions are
unbounded.

### 3. `Action::pending()` answers “is it running?”

`Action::value()` is the latest completed result. After the first successful
run, a later dispatch may leave the previous `Some(...)` value visible while the
new request is pending. Use `pending()` for loading state; do not infer pending
state from `value().is_none()`.

The live-search UI also de-duplicates debounce and explicit-submit dispatches and
hides stale results when the query is cleared.

### 4. Add global Axum layers after all routes they must cover

`Router::layer()` applies to routes already present on that router. Build/nest
Leptos routes, health endpoints, metrics, and fallback first, then apply global
cross-cutting middleware such as `TraceLayer`.

The live-search and i18n routers follow this ordering.

### 5. Serve real hydration assets

SSR HTML can look correct while the browser is completely non-interactive if
its JS/WASM/CSS is missing. Browser E2E must exercise the production route tree
and real `/pkg/*` build artifacts.

For Stylance, the Rust macro provides typed/hashed class names, while
`stylance-cli` performs the CSS transformation/bundling step. The build must run
both Rust/WASM compilation and `stylance build`.

### 6. One database means one SQLx migration history

Every service sharing the same database and `_sqlx_migrations` table must resolve
the same migration set. This repository therefore has one root `migrations/`
directory used by live-search, gateway, and testcontainers fixtures.

Do not create service-local migration directories against the same database and
do not bypass SQLx bookkeeping in tests with raw DDL.

### 7. LISTEN/NOTIFY is a wakeup channel, not durable delivery

PostgreSQL notifications can be missed while a listener is disconnected. The
canonical live-search path therefore:

1. assigns each inserted result a durable `event_seq`,
2. serializes sequence assignment through a transaction-scoped advisory lock so
   visible sequence order matches commit order,
3. uses NOTIFY for low-latency wakeups,
4. pages all rows with `event_seq > last_seen_seq` after reconnect,
5. advances the high-water mark during replay and live delivery.

Never use UUIDv4 ordering as a reconnect cursor. A plain sequence/BIGSERIAL is
also insufficient for a high-water cursor when concurrent transactions may
commit out of allocation order unless that ordering assumption is addressed.

For higher-write-volume systems, prefer a dedicated durable outbox/event table
rather than serializing application-table inserts.

### 8. PostgreSQL channel names and SSE event names are separate namespaces

The database channel is `search_results`. Browser SSE uses named events such as
`connected`, `search_result`, and `stream_lagged`. A client subscribing only to
`search_result` cannot receive the other two names.

Do not mark an EventSource connected merely because its constructor succeeded;
wait for a real server/open event.

### 9. Background tasks need cancellation and ownership

Long-lived tasks should be owned by a structured lifecycle:

- `CancellationToken` to signal shutdown,
- `JoinSet` (or equivalent) to own task handles,
- `tokio::select!` to race blocking waits/backoff against cancellation.

Shutdown order for live-search is: signal cancellation, drain owned tasks, then
close the database pool and flush telemetry.

### 10. Separate liveness from readiness

`live-search` exposes a cheap `/health` liveness endpoint and `/readyz`, which
probes PostgreSQL. Compose uses readiness. The readiness query deliberately uses
the raw `PgPool` rather than the application instrumentation wrapper so routine
orchestrator probes do not create database spans.

The gateway aggregate `/health` checks both registered service modules and the
PostgreSQL dependency required for login/refresh-token operation.

`ServiceModule::health_check` is required: a new module must explicitly define
what healthy means instead of inheriting unconditional green status.

### 11. Validate configuration at startup

`crates/config` loads defaults, optional TOML, then `RWF_*` environment
overrides. Nested keys use a double underscore:

```text
RWF_GATEWAY__PORT=4000
RWF_GATEWAY__CORS__ALLOWED_ORIGINS=https://example.com
RWF_GATEWAY__SESSION__COOKIE_SECURE=true
RWF_GATEWAY__SSE_BROADCAST_BUFFER=512
RWF_LIVE_SEARCH__POOL_MAX_CONNECTIONS=50
```

The loader rejects zero channel capacities, invalid pool bounds, empty required
URLs, zero timeouts, and token TTLs that cannot fit the signed duration used by
the gateway.

Security booleans must be parsed explicitly. Do not make an unrecognized string
silently mean `false`.

Secrets remain deployment environment variables (`JWT_PRIVATE_KEY_PEM`,
`JWT_PUBLIC_KEY_PEM`, `ADMIN_PASSWORD`, `DATABASE_URL`, etc.).

### 12. Demo authentication must not allow caller-chosen subjects

The gateway's deliberately tiny password demo binds `ADMIN_PASSWORD` to one
configured `ADMIN_USER_ID`. Knowing the password is not permission to mint a JWT
for an arbitrary syntactically valid UUID.

A production application should replace this demo pair with a real user store
and password hashing (for example Argon2id), not expand the shared-password
pattern.

Access JWTs are short-lived. Logout/replay revokes refresh-token state; an
already-issued access token remains valid until its `exp` unless a separate
access-token revocation mechanism is added. Do not claim `jti` alone provides
revocation.

### 13. Refresh-token rotation is transactional

Raw refresh tokens are random opaque secrets; only their digest is stored. The
rotation transaction locks the presented active row, creates a replacement,
sets `replaced_by`, revokes the old row, and inserts the replacement atomically.
A replay of an already-used token revokes the whole token family.

`hashed_token` is database-unique so one presented credential cannot map to
multiple rows.

Strict family revocation means two simultaneous legitimate refresh attempts can
invalidate the family. That is a deliberate security/UX tradeoff and should be
documented if this pattern is reused.

### 14. Domain newtypes must enforce their invariant on every public constructor

`rwf_domain::UserId` rejects the nil UUID. Do not expose a public tuple field or
unchecked constructor that lets callers bypass the same invariant enforced by
`TryFrom`, parsing, or Serde.

### 15. Do not leak implementation errors to clients

Log database/upstream/internal errors with `tracing`, then return stable generic
client messages. Raw `sqlx::Error`/upstream parser messages must not be embedded
in public server-function or JSON responses.

Map upstream HTTP failures as gateway failures (`502`/`503` as appropriate), not
unexplained internal `500`s.

## Search pattern

`live-search` combines PostgreSQL full-text search with a pg_trgm title fallback.
The trigram index exists because the query actually uses `title % $1` and
`similarity(...)`; do not advertise typo tolerance while running FTS only.

Search query limits count Unicode scalar values (`chars().count()`), not UTF-8
bytes, when the error message says “characters”.

The Moka cache is invalidated when search-result change events are delivered or
replayed. If the table becomes mutable beyond the events covered by the trigger,
notification/invalidation semantics must be expanded at the same time.

## SSE pattern

Server events are typed JSON payloads carried by named SSE events. Browser code
subscribes to every event name it handles. A slow receiver may receive a
`stream_lagged` event; the per-process broadcast channel is not durable history.
Durable reconnect is provided at the PostgreSQL listener layer, not per-browser
`Last-Event-ID` replay.

If browser-level resume across client disconnects becomes required, expose the
durable event sequence as an SSE id and implement `Last-Event-ID` semantics.

## Axum gateway pattern

Each module implements:

```rust
pub trait ServiceModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn path(&self) -> &'static str { self.name() }
    fn description(&self) -> &'static str;
    fn enabled(&self) -> bool { true }
    fn router(&self) -> axum::Router<GatewayState>;
    fn health_check(&self)
        -> futures::future::BoxFuture<'_, Result<(), ServiceHealthError>>;
}
```

Public query/body inputs should be typed DTOs. The proxy check uses a typed
`IpAddr`; login and refresh use typed validated request structs.

The default rate limiter intentionally keys direct deployments by peer IP. Do
not trust `X-Forwarded-For`/similar headers until the deployment has a trusted
proxy boundary that strips attacker-supplied forwarding headers.

## Sessions, CORS, CSRF, CSP

The gateway demonstrates both Bearer-token auth and a cookie session. Keep the
threat models distinct:

- session cookie is `HttpOnly` and configurable `Secure`,
- credentialed CORS uses explicit origins/methods/headers rather than wildcards,
- mutating session routes are CSRF protected,
- login/refresh bootstrap routes are intentionally not synchronizer-token
  protected because they establish/use other credentials,
- CSP is set without overwriting an existing policy.

The development Compose profile explicitly disables the Secure cookie flag
because it serves plain localhost HTTP. Production defaults secure.

## OpenTelemetry

The `otel` features use OTLP/HTTP. `OTEL_EXPORTER_OTLP_ENDPOINT` defaults to
`http://127.0.0.1:4318` when unset. Axum OTel middleware is required to extract
incoming W3C `traceparent`/`tracestate`; configuring a global propagator alone
does not parent request spans.

`live-search` enables `sqlx-otel` 0.5 under the active `otel` feature. The
request-facing `AppPool` is an instrumented `sqlx_otel::Pool<Postgres>` wrapping
a clone of the same underlying `PgPool`, so enabling tracing does not create a
second physical connection pool or change the configured connection budget.
Migrations, LISTEN/NOTIFY, readiness, and graceful shutdown continue to use the
raw `PgPool` because those paths need concrete SQLx APIs or should avoid noisy
probe spans.

The SQLx integration is deliberately **traces only** today. No OpenTelemetry
`MeterProvider` is installed, so `sqlx-otel`'s optional runtime pool-metrics
feature is not enabled. `axum-prometheus` is a separate metrics pipeline; it
does not automatically receive native OpenTelemetry metrics. Add a real OTel
meter provider before enabling SQLx OTel metrics.

CI includes a focused integration test using a real Postgres testcontainer and
an in-memory OTel exporter. It executes a query through the production AppPool
wrapper beneath an active `tracing-opentelemetry` parent and asserts that the
emitted database `CLIENT` span shares the trace and has the request span as its
parent.

Metrics and trace export remain separate concerns even when they are gated by
nearby observability features.

## WebSocket demo

`i18n-demo/src/ws_chat.rs` is an intentionally small in-memory hub. It still
applies basic browser/server hygiene:

- same-origin browser upgrades are checked using `Origin` vs `Host`,
- Axum `max_message_size` and `max_frame_size` reject oversized payloads before
  application-level buffering,
- the sending connection does not receive its own event,
- non-browser clients without `Origin` remain supported.

For authenticated production WebSockets, validate Origin against a configured
allowlist and wire active connections into application shutdown.

## i18n demo

Locale keys are compile-time checked through generated `leptos_i18n` macros.
The document starts as English SSR and the hydrated client updates `<html lang>`
when the locale changes. Use a dedicated translation key for the language
switch control; do not reuse unrelated UI keys merely to exercise the macro.

Persisted locale choice and Accept-Language negotiation are intentionally out of
scope for this minimal example.

## Islands

If using Leptos islands on the pinned 0.8 line, enable Leptos's `islands` feature
and mark island components accordingly. Do not document a nonexistent
`experimental-islands` feature.

## chromiumoxide E2E

`Browser::launch` does not require chromiumoxide's `bytes` feature. Browser tests
need a Chrome/Chromium executable (`CHROME_PATH` in this repository) and should
use a unique profile directory when relevant.

The `bytes` feature should only be enabled for APIs that actually require it;
do not cite it as a prerequisite for launching the browser.

Tests must fail visibly when required infrastructure is missing. Do not silently
skip a browser/database test after deciding it is part of CI.

## Docker/Compose

Runtime images are non-root and explicitly create `/app`; do not rely on distro
`useradd` home-directory side effects.

The production gateway Compose service requires JWT key material and the admin
password from the host environment and has database-backed refresh storage. For
local exploration, use the `gateway-dev` profile, which requires
`ALLOW_DEV_KEYS=1` and generates ephemeral Ed25519 keys.

Example local stack:

```bash
docker compose --profile dev up --build \
  postgres live-search i18n-demo gateway-dev
```

## CI expectations

The maintained GitHub Actions workflow should cover:

- native workspace/all-target Clippy plus supported native feature combinations,
- SSR and hydration feature matrices,
- the `otel` feature compilation,
- Rust formatting and Leptos view formatting,
- dependency audit,
- Leptos production builds,
- all Dockerfile builds using a Docker daemon,
- browser E2E with an installed Chromium binary,
- a runtime SQLx OTel span-parenting check against real PostgreSQL.

`--locked` protects the committed dependency graph. CI-installed helper tools
must also pin their top-level versions so a crates.io release cannot silently
change the workflow's toolchain or MSRV requirements.

## References

The code is the primary reference. Supporting prose lives in `references/`, but
it must not override the implementation or current upstream behavior.
Particularly useful entry points:

- `live-search/src/bootstrap.rs`
- `live-search/src/db.rs`
- `live-search/src/app.rs`
- `gateway/src/gateway.rs`
- `gateway/src/auth/refresh.rs`
- `gateway/src/settings.rs`
- `e2e-tests/src/common/live_search_env.rs`
- `e2e-tests/tests/live_search_test.rs`
- `.github/workflows/ci.yml`
