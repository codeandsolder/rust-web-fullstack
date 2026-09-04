# rust-web-fullstack

A reference workspace for **Leptos 0.8 + Axum 0.8 + PostgreSQL/sqlx 0.9**.
It is intentionally small enough to study, but exercises the failure modes that
toy examples usually omit: SSR + hydration assets, durable recovery around
LISTEN/NOTIFY, rotating refresh tokens, typed configuration, readiness checks,
Docker builds, and real browser E2E tests.

## Workspace

| Path | What it demonstrates |
|---|---|
| `live-search/` | Leptos SSR/hydration, PostgreSQL FTS + pg_trgm search, Moka cache, PgListener, SSE |
| `gateway/` | composable Axum services, Ed25519 JWTs, rotating refresh tokens, sessions/CSRF/CORS/CSP, OpenAPI |
| `i18n-demo/` | compile-time-checked EN/DE translations and a hardened minimal WebSocket hub |
| `e2e-tests/` | isolated Postgres via testcontainers plus chromiumoxide browser tests |
| `crates/config/` | validated TOML + `RWF_*` environment configuration |
| `crates/domain/` | framework-free domain types and invariants |
| `migrations/` | the **single** SQLx migration history used by every service sharing the database |

The workspace uses Edition 2024 and pins Rust **1.94** as its supported toolchain/MSRV.

## Quick start

```bash
git clone https://github.com/codeandsolder/rust-web-fullstack.git
cd rust-web-fullstack

docker compose --profile dev up --build \
  postgres live-search i18n-demo gateway-dev
```

Services:

- live-search: <http://localhost:3000>
- gateway-dev: <http://localhost:3001>
- i18n-demo: <http://localhost:3002>
- PostgreSQL: `localhost:5432`, database `rwf_demo`

The dev gateway uses ephemeral Ed25519 keys and is deliberately guarded by
`ALLOW_DEV_KEYS=1`. It also disables the session cookie's `Secure` flag because
the Compose dev endpoint is plain localhost HTTP. Do not copy those two settings
into production.

## Production gateway configuration

The normal `gateway` Compose service fails fast unless required secrets are
provided:

```bash
export JWT_PRIVATE_KEY_PEM='-----BEGIN PRIVATE KEY----- ...'
export JWT_PUBLIC_KEY_PEM='-----BEGIN PUBLIC KEY----- ...'
export ADMIN_PASSWORD='a-long-demo-password'
export ADMIN_USER_ID='00000000-0000-0000-0000-000000000001' # optional override

docker compose up --build postgres live-search gateway
```

The password demo is intentionally bound to exactly one `ADMIN_USER_ID`. Knowing
the demo password is **not** permission to choose an arbitrary UUID as a JWT
subject. A real application should replace this credential pair with a user
store and password hashing.

Access JWTs are short-lived (15 minutes by default). Refresh tokens are random
opaque values; only their hashes are stored. Rotation is transactional, records
replacement lineage, and revokes the whole token family if an already-used token
is replayed. `/auth/logout` revokes every outstanding refresh token for the
authenticated subject and flushes only the cookie session attached to that
request; other cookie sessions for the same subject are not enumerated. An
already-issued access JWT remains valid until its `exp` unless you add a separate
access-token revocation system. `/session/logout` only flushes the current cookie
session and does not revoke refresh-token state.

## Configuration

`crates/config` loads:

1. built-in defaults,
2. `config.toml` (or `RWF_CONFIG`),
3. `RWF_*` environment overrides.

Nested keys use `__`:

```bash
RWF_GATEWAY__PORT=4000
RWF_GATEWAY__CORS__ALLOWED_ORIGINS=https://example.com
RWF_GATEWAY__SESSION__COOKIE_SECURE=true
RWF_GATEWAY__SSE_BROADCAST_BUFFER=512
RWF_LIVE_SEARCH__POOL_MAX_CONNECTIONS=50
RWF_LIVE_SEARCH__SSE_BROADCAST_BUFFER=512
```

The loader validates cross-field invariants such as nonzero broadcast capacity,
positive timeouts/TTLs, and `pool_min_connections <= pool_max_connections`.
Malformed security booleans fail startup instead of silently weakening a setting.

Secrets such as `DATABASE_URL`, JWT keys, and the demo password remain deployment
environment variables.

## One migration history

Both live-search and gateway use the same PostgreSQL database in Compose, so they
must also resolve the same SQLx migration set. The root `migrations/` directory is
the only authoritative history and is used by both binaries and testcontainers.

Do not add service-local SQLx migration directories against the same database;
SQLx will otherwise see the other service's applied versions as missing.

## Live search and durable event recovery

Search combines PostgreSQL full-text search with a pg_trgm title fallback, so the
trigram index is part of the actual query rather than decorative schema.

PostgreSQL LISTEN/NOTIFY is used for low-latency wakeups, **not** durable delivery.
Reconnect recovery is driven by a durable `event_seq`:

- inserts receive a transaction-serialized sequence value,
- sequence order therefore matches visible commit order,
- reconnect replay pages every row newer than the last delivered sequence,
- the cursor advances during replay and normal notification delivery.

This avoids the two classic broken approaches: ordering UUIDv4 values, and using
a plain sequence as a high-water mark without accounting for concurrent commit
order.

The browser SSE feed uses named events (`connected`, `search_result`,
`stream_lagged`). The client subscribes to each name it handles and does not mark
itself connected merely because `EventSource::new()` succeeded.

`/health` is process liveness. `/readyz` probes PostgreSQL and is what Compose
uses for live-search readiness.

## Gateway health and proxy behavior

`ServiceModule::health_check` is required, so a new module cannot accidentally
inherit an unconditional green probe. The aggregate gateway `/health` also checks
PostgreSQL because login/refresh functionality requires it.

The proxy example accepts a typed `IpAddr`; upstream transport/status/parse
failures are logged internally and returned as `502 Bad Gateway` rather than
leaking implementation errors as generic 500 responses.

Rate limiting uses peer IP by default. That is deliberate: forwarded-IP headers
must not become trusted limiter keys until a deployment has a trusted reverse
proxy that strips attacker-supplied forwarding headers.

## Sessions, CORS, CSRF, CSP

The gateway demonstrates a cookie session alongside Bearer JWTs:

- session cookies are `HttpOnly` and `Secure` by default,
- credentialed CORS uses explicit origins/methods/headers rather than wildcards,
- mutating session routes are CSRF protected,
- session-store failures return 5xx instead of HTTP 200 error payloads,
- CSP is added without overwriting an existing policy.

The session backend in this reference implementation is `tower_sessions::MemoryStore`.
Its state is process-local: a gateway restart invalidates sessions and multiple
gateway replicas do not share them. Use a shared session store for horizontally
scaled production deployments that need stable server-side sessions.

The two auth styles are examples of different threat models, not an argument that
every endpoint should use both at once.

## OpenTelemetry

The optional `otel` features export traces over OTLP/HTTP. If
`OTEL_EXPORTER_OTLP_ENDPOINT` is unset, the examples default to
`http://127.0.0.1:4318`.

Both Axum routers install OpenTelemetry middleware that extracts incoming W3C
`traceparent`/`tracestate`; setting a global propagator alone would not do that.

`live-search` enables `sqlx-otel` 0.5 under its `otel` feature. Request-facing
application queries use an instrumented `sqlx_otel::Pool<Postgres>` that wraps a
clone of the same underlying SQLx pool; it does **not** create a second physical
connection pool. Migrations, LISTEN/NOTIFY, readiness probes, and graceful
shutdown retain the raw `PgPool` for SQLx-specific APIs and to avoid noisy probe
spans.

This integration currently exports database **traces only**. live-search does
not install an OpenTelemetry `MeterProvider`, so `sqlx-otel`'s optional runtime
pool-metrics task is intentionally not enabled. The existing Axum Prometheus
layer is a separate metrics pipeline and does not receive native OTel metrics.
CI includes a focused real-Postgres test that verifies a SQLx `CLIENT` span is
emitted beneath an active tracing/OpenTelemetry parent span.

## i18n and WebSocket example

The i18n demo has compile-time-checked EN/DE translation keys. SSR starts in
English and the hydrated client updates the document `<html lang>` attribute when
the locale changes.

The WebSocket chat is still only an in-memory demo, but it demonstrates minimum
browser/server hygiene:

- same-origin browser upgrades (`Origin` vs `Host`),
- protocol-level 1 KiB frame/message limits,
- no sender echo,
- explicit lag handling.

A production authenticated WebSocket service should use a configured Origin
allowlist, durable/shared messaging if horizontally scaled, and application
shutdown wiring for active connections.

## Build and validation

Basic static/test pass:

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
```

SSR/hydration production builds:

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos --locked
cargo install stylance-cli --locked

cargo leptos build --release -p live-search
stylance live-search --output-file live-search/target/site/pkg/live-search.css
cargo leptos build --release -p i18n-demo
```

Stylance's Rust macro provides typed/hashed class names; `stylance-cli` is what
actually transforms/bundles CSS. It is therefore a separate build step.

### Browser E2E

Browser tests mount the real Leptos SSR route tree and use real build artifacts;
they do not substitute a text-only fixture for the frontend.

Requirements: Docker daemon, Chromium/Chrome, and the live-search Leptos build.
The helper script performs the asset build and prerequisite checks:

```bash
CHROME_PATH=/usr/bin/chromium ./scripts/test-e2e.sh
```

Or invoke the test binary directly after building assets:

```bash
LIVE_SEARCH_PKG_DIR=live-search/target/site/pkg \
CHROME_PATH=/usr/bin/chromium \
cargo test --release --locked -p e2e-tests --tests \
  --features browser-tests -- --test-threads=1 --nocapture
```

Tests fail when required infrastructure/artifacts are missing rather than silently
skipping the coverage CI claims to provide.

## CI

`.github/workflows/ci.yml` is the executable CI reference. It runs on pull
requests and pushes to `main` and covers lockfile consistency, Rust and Leptos
formatting, native Clippy feature combinations, WASM hydration checks, workspace
library tests, `cargo audit`, production Leptos builds, all three Docker images,
and real Chromium-backed E2E tests.

The workflow pins Rust 1.94 and the helper tool versions it installs. Expensive
Rust jobs reuse compiler state through `actions/cache`, while Docker builds use
per-image BuildKit GitHub Actions layer caches. Keep CI feature coverage and cache
keys in sync whenever supported features, the Rust toolchain, or build tooling
change.

## Canonical development notes

The maintained implementation notes live in [`SKILL.md`](SKILL.md). The most
important code entry points are:

- `live-search/src/bootstrap.rs`
- `live-search/src/db.rs`
- `live-search/src/app.rs`
- `gateway/src/gateway.rs`
- `gateway/src/auth/refresh.rs`
- `gateway/src/settings.rs`
- `e2e-tests/src/common/live_search_env.rs`
- `e2e-tests/tests/live_search_test.rs`
- `.github/workflows/ci.yml`

If documentation and implementation diverge, fix both in the same change.
