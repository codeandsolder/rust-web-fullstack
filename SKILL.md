---
name: rust-web-fullstack
description: Full-stack Rust web development with Leptos 0.8.x, PostgreSQL via sqlx, axum, SSE streaming, and LISTEN/NOTIFY live queries. Use this skill when building Rust web apps, connecting Leptos to databases, implementing live updates, working with SSR/CSR/hydration, setting up SSE endpoints, using sqlx PgListener, writing E2E tests with chromiumoxide, or doing visual testing with Chrome DevTools MCP. Trigger on mentions of Leptos, sqlx, axum, tower-http, tower, axum middleware, PostgreSQL, SSE, LISTEN/NOTIFY, live queries, pg_notify, SSR, CSR, hydration, cargo-leptos, LeptosRoutes, server functions, PgPool, PgListener, broadcast channel, EventSource, chromiumoxide, JWT, session, HttpOnly, WASM, wasm-bindgen, hydrate_body, Cargo workspace, [workspace.lints], Edition 2024, graceful shutdown, signal handling, SIGTERM, tracing_subscriber, RUST_LOG, EnvFilter, tracing spans, #[instrument], nextest, insta, rstest, mockall, criterion, proptest, CancellationToken, JoinSet, tokio::select!, leptos_i18n, leptosfmt, leptos-use, leptos-sse, leptos-declarative, leptos_oidc, leptos_ws, leptos-material, leptos-tea, tailwind-fuse, t! macro, compile-time-checked translations, view! macro formatter, or full-stack Rust architecture.
---

# Rust Web Fullstack — Leptos + PostgreSQL + Axum
## Contents

### Quick Reference
- [Crate Versions](#crate-versions)
- [Architecture Decision Tree](#architecture-decision-tree)
- [When NOT to use this skill](#when-not-to-use-this-skill)
- [Critical Rules](#critical-rules)
  - [1. Feature flags are mutually exclusive per build target](#1-feature-flags-are-mutually-exclusive-per-build-target)
  - [2. Leptos 0.8 Action state](#2-leptos-08-action-state)
  - [3. PgListener consumes 1 connection from the pool](#3-pglistener-consumes-1-connection-from-the-pool)
  - [4. `render_app_to_stream_with_context` creates a fresh reactive tree per request](#4-render_app_to_stream_with_context-creates-a-fresh-reactive-tree-per-request)
  - [5. SSE auto-headers](#5-sse-auto-headers)
  - [6. Server fn path doubling](#6-server-fn-path-doubling)
  - [7. Static files for hydration](#7-static-files-for-hydration)
  - [8. chromiumoxide SingletonLock](#8-chromiumoxide-singletonlock)
  - [9. Integration tests must fail visibly](#9-integration-tests-must-fail-visibly)
  - [10. Background tasks need structured-concurrency wiring](#10-background-tasks-need-structured-concurrency-wiring)
  - [11. Postgres channel name vs EventSource event name are distinct namespaces](#11-postgres-channel-name-vs-eventsource-event-name-are-distinct-namespaces)

### Workspace Setup
- [`[workspace.lints]` Table](#workspacelints-table)
- [View! Macro Formatting (`leptosfmt`)](#view-macro-formatting-leptosfmt)
- [Tracing Subscriber Init](#tracing-subscriber-init)
- [Structured Fields, Not Interpolation](#structured-fields-not-interpolation)
- [Reference Files](#reference-files)

### Patterns
- [Pattern 1: Leptos SSR + Axum + PostgreSQL](#pattern-1-leptos-ssr-axum-postgresql)
- [Pattern 2: Live Updates via LISTEN/NOTIFY → broadcast → SSE](#pattern-2-live-updates-via-listennotify-broadcast-sse)
- [Pattern 3: PostgreSQL FTS with tsvector/tsquery](#pattern-3-postgresql-fts-with-tsvectortsquery)
- [Pattern 4: E2E Test with chromiumoxide](#pattern-4-e2e-test-with-chromiumoxide)
- [Pattern 5: JSONB Storage with Compile-Time Checking](#pattern-5-jsonb-storage-with-compile-time-checking)
- [Pattern 6: Gateway with ServiceModule Trait](#pattern-6-gateway-with-servicemodule-trait)
- [Pattern 7: JavaScript-Driven SSE Detection (for Chrome DevTools MCP)](#pattern-7-javascript-driven-sse-detection-for-chrome-devtools-mcp)
- [Pattern 8: TTL Cleanup via pg_cron](#pattern-8-ttl-cleanup-via-pgcron)
- [Pattern 9: Server-Fn Catch-All Route (Leptos 0.8 Doubled-Prefix Bug)](#pattern-9-server-fn-catch-all-route-leptos-08-doubled-prefix-bug)
- [Pattern 10: SSR + Hydration Setup (Same Crate as Both Bin & Lib)](#pattern-10-ssr--hydration-setup-same-crate-as-both-bin--lib)
- [Scoped CSS with stylance](#scoped-css-with-stylance)
- [Pattern 11: Action.value() vs Action.input()](#pattern-11-actionvalue-vs-actioninput)
- [Pattern 12: chromiumoxide E2E Helpers](#pattern-12-chromiumoxide-e2e-helpers)
- [Pattern 13: chromiumoxide Chrome Binary Selection](#pattern-13-chromiumoxide-chrome-binary-selection)
- [Pattern 14: SSE JSON Injection in Rust Raw Strings](#pattern-14-sse-json-injection-in-rust-raw-strings)
- [Pattern 15: Structured Concurrency Triad (CancellationToken + JoinSet + select!)](#pattern-15-structured-concurrency-triad-cancellationtoken-joinset-select)
- [Pattern 16: Newtype IDs for Type-Safe Web Params](#pattern-16-newtype-ids-for-type-safe-web-params)
- [Cross-cutting Concerns: CSRF, CSP, CORS](#cross-cutting-concerns-csrf-csp-cors)
- [Pattern 17: Modular Server Lifecycle (`bootstrap::run` + `shutdown::wait`)](#pattern-17-modular-server-lifecycle-bootstraprun--shutdownwait)
- [Pattern 18: Layered config (TOML file + RWF_* env overrides)](#pattern-18-layered-config-toml-file-rwf_-env-overrides)
- [Pattern 19: Leptos 0.8.x Knowledge Patch](#pattern-19-leptos-08x-knowledge-patch)
- [Pattern 20: Leptos Utility Ecosystem](#pattern-20-leptos-utility-ecosystem)
- [Pattern 21: ErrorBoundary + ActionForm (Progressive Enhancement)](#pattern-21-errorboundary--actionform-progressive-enhancement)
- [Pattern 22: Cursor-Based Pagination](#pattern-22-cursor-based-pagination)
- [Pattern 23: Leptos Islands Architecture](#pattern-23-leptos-islands-architecture)
- [Pattern 24: Atomic Refresh-Token Rotation (PostgreSQL)](#pattern-24-atomic-refresh-token-rotation-postgresql)
- [Pattern 25: WebSocket Chat via static broadcast hub](#pattern-25-websocket-chat-via-static-broadcast-hub)

### Common Pitfalls
- [Pitfall 1: PgListener connection leak](#1-pglistener-connection-leak)
- [Pitfall 2: Broadcast channel overflow](#2-broadcast-channel-overflow)
- [Pitfall 3: Leptos SSR hangs](#3-leptos-ssr-hangs)
- [Pitfall 4: JSONB in sqlx macros](#4-jsonb-in-sqlx-macros)
- [Pitfall 5: Feature flag conflicts](#5-feature-flag-conflicts)
- [Pitfall 6: cross-origin SSE](#6-cross-origin-sse)
- [Pitfall 7: chromiumoxide user_data_dir collision](#7-chromiumoxide-user_data_dir-collision)
- [Pitfall 8: WASM hydration requires static serving](#8-wasm-hydration-requires-static-serving)
- [Pitfall 9: Server-fn 404 / doubled-prefix](#9-server-fn-404--doubled-prefix)
- [Pitfall 10: jsonwebtoken 10 panics without crypto provider](#10-jsonwebtoken-10-panics-without-crypto-provider)
- [Pitfall 11: Silent test skips via check_server_or_skip()](#11-silent-test-skips-via-check_server_or_skip)
- [Pitfall 12: Stale target/debug/deps/ fingerprints](#12-stale-targetdebugdeps-fingerprints)
- [Pitfall 13: sccache is local-disk by default](#13-sccache-is-local-disk-by-default)
- [Pitfall 14: Background tasks missing CancellationToken wiring](#14-background-tasks-missing-cancellationtoken-wiring)
- [Pitfall 15: `#[server]` body not cfg-gated on `feature = "ssr"`](#15-server-body-not-cfg-gated-on-feature--ssr)

### Reference
- [Test Strategy](#test-strategy)
- [Bundle of Patterns (for AI model loading)](#bundle-of-patterns-for-ai-model-loading)


## Canonical Reference Implementation

This skill ships with a complete, runnable reference workspace next to it. Every pattern in this skill is implemented and verified in that code.

| Path | What it shows |
|------|---------------|
| `./live-search/src/main.rs` | Pattern 15 (full triad): `CancellationToken` + `JoinSet` + signal handler + `tokio::select!` for `axum::serve` shutdown |
| `./live-search/src/db.rs::run_pg_listener` | Critical Rule 10 + Pitfall 14: `PgListener::recv()` raced against cancellation, with cancellable backoff sleep |
| `./live-search/src/app.rs::search` | Pitfall 15: the `#[server]` body is cfg-gated `#[cfg(feature = "ssr")]` so `cargo check --workspace --all-targets` compiles without the `ssr` feature |
| `./live-search/src/app.rs::LiveFeedPage` | Pattern 2 client: hand-rolled `gloo-net` EventSource reconnect loop, `Arc<str>` payload ring buffer of 200, named-event subscription via `subscribe("search_result")` |
| `./gateway/src/main.rs` | Pattern 15's shutdown primitive only (no spawned tasks → no `JoinSet` / `CancellationToken` required) |
| `./gateway/src/module.rs::ServiceHealthError` | `#[non_exhaustive]` + `#[must_use]` + doc comment design pattern |
| `./i18n-demo/src/i18n.rs` + `./i18n-demo/locales/{en,de}.json` + `./i18n-demo/build.rs` | Compile-time-checked internationalization with `leptos_i18n` 0.6 — JSON locales loaded by `leptos_i18n_build`, `t!` macro for key-checked interpolation, runtime locale switching via `I18nContext::set_locale` |
| `./i18n-demo.Dockerfile` + `./docker-compose.yml` `i18n-demo` service on `:3002` | Same multi-stage Leptos build pattern as `live-search`, dedicated port (Postgres-free crate) |
| `./gateway.Dockerfile` + `./live-search.Dockerfile` + `./docker-compose.yml` | Multi-stage Leptos build with `cargo-leptos`, runtime slim image, Postgres + pgAdmin + Chromium |
| `./e2e-tests/` | chromiumoxide-based Playwright replacement for browser-driven E2E |
| `./.woodpecker.yml` `fmt` step (existing) + `leptosfmt` step (per-file piped form, see below) | Edition 2024 + Rust 1.94 + `--all-targets` + strict clippy with `-D warnings` + per-file `leptosfmt --rustfmt --stdin <file>` for `view!` macro bodies, then `cargo fmt --check` as authoritative whitespace check |
| `./Cargo.toml` | Workspace Edition 2024 with strict `[workspace.lints]` table |

### Prerequisites

The reference workspace needs PostgreSQL on `localhost:5432` for `live-search`
and `gateway`. The canonical `docker-compose.yml` at the workspace root ships a
Postgres + pgAdmin + Chromium stack wired to the right ports and credentials:

```bash
cd ~/.config/opencode/skills/rust-web-fullstack
docker compose up -d postgres                    # just the DB
# or:
docker compose up -d                             # full stack including pgAdmin + Chromium
```

For `cargo leptos build` (the SSR + hydrate WASM build), install
[`cargo-leptos`](https://github.com/leptos-rs/cargo-leptos) once:

```bash
cargo install cargo-leptos --locked
```

For the `view!` macro formatter:

```bash
cargo install leptosfmt --locked
```

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

### Crate Versions

Last verified against the canonical `Cargo.lock` in this directory. Re-verify
after any `cargo update` or workspace member addition.

| Crate | Version | Features | Notes |
|-------|---------|----------|-------|
| `leptos` | 0.8 | `csr`, `ssr`, `hydrate` — mutually exclusive per build target | 0.9 is alpha; stay on 0.8.x |
| `leptos_axum` | 0.8 | SSR integration with axum | Doubles `/api` prefix on server fns — see Pitfall 9 |
| `sqlx` | 0.9 | `postgres`, `runtime-tokio`, `tls-rustls`, `json`, `macros`, `migrate` | Canonical workspace uses *runtime* queries only (`sqlx::query_as::<_, T>(…)`); compile-time `query!` requires `cargo sqlx prepare` + `.sqlx/` cache |
| `axum` | 0.8 | `json` | |
| `tokio` | 1 | `full` | `JoinSet` and `tokio::time::Instant` come from `tokio` directly |
| `tokio-util` | 0.7 | `["rt"]` | `CancellationToken` lives in `tokio_util::sync` (always compiled, no feature needed); `rt` is for `task::JoinMap` and is included here because the workspace uses it elsewhere — drop it if you don't need `JoinMap` |
| `tower-http` | 0.7 | workspace enables `["trace", "cors", "fs", "timeout"]`; `live-search` and `i18n-demo` add `fs` for `ServeDir`; `gateway` adds `set-header` for `SetResponseHeaderLayer` (currently documented but not yet imported — see `gateway/src/cors.rs:80-83` for the `axum 0.8 Body` compatibility note) | Per-crate feature lists are additive on top of the workspace set; drop a feature from a crate's own `[dependencies] table` to disable it there |
| `jsonwebtoken` | 10 | `["aws_lc_rs"]` (workspace choice) | 10.x panics without explicit crypto provider — see Pitfall 10 |
| `reqwest` | 0.13 | `default-features = false`, `rustls`, `json`, `stream` | `default-features = false` avoids the `native-tls` conflict with `rustls`; `stream` enables `bytes_stream()` for SSE reading |
| `chromiumoxide` | 0.9 | `default-features = false`, `bytes` | `default-features = false` keeps the tokio version compatible with the workspace pin; `bytes` is required for `Browser::launch(...)` |
| `gloo-net` | 0.7 | `eventsource` | Client-side SSE reader |
| `leptos_i18n` | 0.6 | `csr` + `hydrate` + `ssr` (all three required for full-stack i18n) | Workspace's `i18n-demo` crate wires these into the `ssr` and `hydrate` Cargo features. The `leptos_i18n_build` build-dep (same version) emits the typed `t!` / `t_string!` / `Locale` modules at compile time. Note: `leptos_i18n 0.6.2` transitively pulls `leptos-use 0.18.3` — harmless, that version is only compiled for the i18n-demo target. |
| `gloo-timers` | 0.4 | `futures` | `gloo_timers::future::sleep` requires the `futures` feature |
| `tracing` | 0.1 | (default) | Structured logging — never `println!` or `log` |
| `tracing-subscriber` | 0.3 | `env-filter`, `fmt` | `env-filter` required to read `RUST_LOG`; install once in `main` — see [Tracing Subscriber Init](#tracing-subscriber-init) below |
| `lepticons` | 0.13 | (default) | Lucide icon set for Leptos |
| `chrono` | 0.4 | `serde` | Timestamp arithmetic and serde roundtripping for `DateTime<Utc>` |
| `config` | 0.15 | `default-features = false`, `toml` | Layered workspace config in `crates/config` (TOML file + `RWF_*` env overrides) — see [Pattern 18](#pattern-18-layered-config-toml-file-rwf_-env-overrides). Earlier versions of the workspace used 0.14; migrated in this revision. |
| `leptos-use` | 0.19 | (default) | `live-search::SearchPage` uses `watch_debounced` for the debounced search box. Two `leptos-use` versions coexist in `Cargo.lock` (`0.18.3` transitive from `leptos_i18n 0.6.2`, `0.19.0` direct) — harmless, each compiles on its own target. |
| `sqlx-otel` | 0.3 | (default) | Optional, behind `live-search`'s `otel` feature. **Compatibility caveat:** `sqlx-otel 0.3.0` pulls `sqlx 0.8.6` + `opentelemetry 0.31.0` alongside the workspace's `sqlx 0.9` + `opentelemetry 0.32`. Do not enable `--features otel` on `live-search` until upstream `sqlx-otel` ships a sqlx-0.9-compatible release. |
| `tower_governor` | 0.8 | `axum` | Rate limiting on `gateway` auth routes (two governor instances for login vs. refresh). **Note:** `tower_governor 0.8 → governor 0.10.4 → {rand 0.9.4, getrandom 0.3.4, web-time}` — the `getrandom 0.3.4` you may see in `cargo tree` comes from here, NOT from `leptos-use`. Compiles fine for the SSR binary. |
| `moka` | 0.12 | `future` | In-memory search-query cache in `live-search` (60s TTL, 1000 entries). |
| `thiserror` | 2 | (default) | Error derive macros — `ServiceHealthError` (Pattern 6), `UserIdError` (Pattern 16), `RefreshTokenError`, `WsChatError`. |
| `futures` | 0.3 | (default) | `StreamExt` (Pattern 2 SSE), `FutureExt` + `BoxFuture` (Pattern 6 `ServiceModule::health_check`), `SinkExt` + `StreamExt` (Pattern 25 WebSocket). |
| `stylance` | 0.5 | (default) | Compile-time scoped CSS via proc-macro — see `Scoped CSS with stylance` below; no `stylance-cli` build step required. |
| `tokio-stream` | 0.1 | `sync` | `BroadcastStream` wrapper used in Pattern 2's `sse_handler` to expose `broadcast::Receiver` as a `Stream` and surface `RecvError::Lagged` as an explicit `stream_lagged` SSE event. |

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

#### 1. Feature flags are mutually exclusive per build target
`csr`, `ssr`, `hydrate` cannot coexist. A crate's `[features]` table must use `skip_feature_sets` or explicit negative deps to make any two of them fail at `cargo check` time.

#### 2. Leptos 0.8 Action state
Use `action.value()` (the action's result) not `action.input()` (the dispatched input) when rendering post-action result UI ("No results found.", error banner, success state). Both `input()` and `value()` persist for the lifetime of the `Action`; they differ in what they hold, not in whether they survive completion. See Pattern 11.

#### 3. PgListener consumes 1 connection from the pool
Budget for it in `max_connections` (e.g. `21 = 20 queries + 1 listener`). One `PgListener` holds exactly one connection regardless of how many channels you `listen`/`listen_all` to — adding channels does NOT increase the connection cost.

#### 4. `render_app_to_stream_with_context` creates a fresh reactive tree per request
Context injection is the standard way to share state. Do not put request-scoped resources (`PgPool`, `AppContext`) in a `OnceLock` and rely on them being visible inside `view!` — the tree is built fresh per request.

#### 5. SSE auto-headers
axum's `Sse::new(stream)` automatically sets `Content-Type: text/event-stream` and `Cache-Control: no-cache`. Do NOT add these manually or `SetResponseHeaderLayer` will double-set them and break CORS preflight caches.

#### 6. Server fn path doubling
`leptos_axum::handle_server_fns` mounted at `/api/*fn_name` will register the route at `/api/api/search` when your server fn macro is configured with `endpoint = "/api/search"`. Fix: use a catch-all handler that tries both — see Pattern 9 below.

#### 7. Static files for hydration
SSR pages load WASM via `/pkg/{crate}.js` and `/pkg/{crate}_bg.wasm`. You MUST mount `tower_http::services::ServeDir::new("./pkg")` (relative to server CWD) before hydration works. The Leptos build writes these to `./pkg` next to your `Cargo.toml` during `cargo leptos build`.

#### 8. chromiumoxide SingletonLock
Every test that spawns a browser MUST use a unique `user_data_dir` (e.g. `<pid>-<nanos>`). Default `~/.cache/chromiumoxide-runner/SingletonLock` collides when tests run in parallel.

#### 9. Integration tests must fail visibly
If a required service, browser, database, fixture, or SSE event is missing, panic/assert with the actual status or error. Use `#[ignore]` for intentionally optional slow tests; do not return early and report success.

#### 10. Background tasks need structured-concurrency wiring
`pg_listener_task` and any other long-running `tokio::spawn`'d task MUST accept a `CancellationToken` and race its primary await against `shutdown.cancelled()` via `tokio::select!`. Dropping a `JoinHandle` does not cancel — only `token.cancel()` cooperatively stops the task. See Pattern 15.

*If your binary has no `tokio::spawn` calls (the gateway, for example), `with_graceful_shutdown(graceful_shutdown_signal())` is sufficient and no `CancellationToken` is needed — `Pattern 15` is still relevant as a reference, but only its shutdown primitive applies.*

#### 11. Postgres channel name vs EventSource event name are distinct namespaces
The Postgres `LISTEN` channel (e.g. `"search_results"`) is the SQL identifier for `NOTIFY`; the EventSource event type (e.g. `"search_result"`) is the client-side selector for `addEventListener`. They happen to share a substring by convention but are different identifiers — see `live-search/src/db.rs:141` (LISTEN) and `live-search/src/app.rs:319` (subscribe).

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

### View! Macro Formatting (`leptosfmt`)

`cargo fmt` does **not** format the contents of `view! { ... }` blocks —
that is `leptosfmt`'s job. Without it, CI's `cargo fmt --check` step
silently passes while `view!` blocks drift in style.

Install once with `cargo install leptosfmt --locked`, then format the
workspace. **`leptosfmt --check .` is NOT used by CI** — the two formatters
disagree on whitespace inside `view!` macro bodies, so `leptosfmt --check .`
and `cargo fmt --check` cannot both pass simultaneously. CI uses the
per-file piped form below so the two formatters converge:

```bash
# Workspace helper (equivalent to the CI command) — uses the Makefile target.
make fmt-all
```

The canonical `.woodpecker.yml` uses the per-file piped form on a custom
Docker image (with `leptosfmt` preinstalled):

```yaml
  fmt:
    image: rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55
    commands:
      - cargo fmt --all -- --check

  leptosfmt:
    image: woodpecker-rust-leptosfmt:latest
    commands:
      # Scope to workspace crates explicitly. `leptosfmt .` would also walk
      # any in-tree gitignored snapshots (.slim/worktrees/, .opencode/, etc.)
      # which may have stale, intentionally-unformatted copies.
      # Run leptosfmt per-file via stdin (so `--rustfmt` chains rustfmt
      # afterwards) so the two formatters converge on whitespace inside
      # `view!` macro bodies.
      - find live-search i18n-demo -name '*.rs' | while read f; do leptosfmt --rustfmt --stdin --quiet < "$f" > "$f.tmp" && mv "$f.tmp" "$f"; done
      - cargo fmt --all -- --check
    depends_on:
      - fmt
```

For editor integration, point `rust-analyzer`'s formatter at it
(via `rust-analyzer.toml` in the repo root):

```toml
[rustfmt]
overrideCommand = ["leptosfmt", "--stdin", "--rustfmt"]
```

#### `leptosfmt --check` round-trip quirk

`leptosfmt --check` is not a perfect superset of `cargo fmt --check`. The
two formatters disagree about whitespace inside `view!` macro bodies in a
small number of cases (notably, leptosfmt preserves trailing whitespace on
comment-only lines that rustfmt strips). Concretely:

- `cargo fmt --all -- --check` will sometimes fail with a tiny diff after
  `leptosfmt --write` has run.
- `leptosfmt --check` will sometimes fail with a tiny diff after
  `cargo fmt --all` has run.

**Resolution**: run `leptosfmt --rustfmt --stdin <file> > file` per file (the
`--rustfmt` flag chains rustfmt after leptosfmt and converges both
formatters' whitespace), then `cargo fmt --all`. The Makefile `fmt-all`
target and `.woodpecker.yml` `leptosfmt` step use this exact sequence; the
final authoritative check is `cargo fmt --all -- --check`.

See [Pattern 20](#pattern-20-leptos-utility-ecosystem) for the full list of
ecosystem tools the skill flags, and Pitfall 15 for the related
`#[server]` cfg-gating issue.

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

## Creating a New Service in This Workspace

A short checklist for adding a new Leptos + axum + PostgreSQL crate that
follows every pattern in this skill. The canonical template is `live-search`
(or `i18n-demo` if you don't need a database).

1. **Workspace member**: add `"new-crate"` to `[workspace] members` in
   `Cargo.toml`.
2. **Inherit lints**: `cat > new-crate/Cargo.toml` with `[lints] workspace = true`
   at the top — every lint below it (panic, expect_used, pedantic) propagates.
3. **Crate type**: set `[lib] crate-type = ["cdylib", "rlib"]` so the same
   crate can compile to both the SSR server and the WASM hydrate bundle.
4. **Binary gate**: `[[bin]]` with `required-features = ["ssr"]` so the
   SSR-only `axum`/`tokio`/`sqlx` deps are never pulled into the WASM build.
5. **Feature gates**: define `ssr = ["dep:leptos_axum", "leptos/ssr"]`,
   `hydrate = ["leptos/hydrate"]`, and `default = []`. Keep them mutually
   exclusive per build target (Critical Rule 1).
6. **Cfg-gate server-only modules**: `#[cfg(feature = "ssr")] pub mod server { ... }`
   for the `PgPool`, `PgListener` task, etc. Add `#[cfg(feature = "ssr")]` to
   the body of every `#[server]` function whose body uses ssr-gated items
   (Pitfall 15).
7. **Provide context**: in the shell closure passed to `leptos_routes`,
   `leptos::context::provide_context(...)` your `AppContext` so SSR components
   see the same state as server fns via `state::get()`.
8. **Static files**: `nest_service("/pkg", ServeDir::new("./pkg"))` before
   `leptos_routes` (Critical Rule 7).
9. **Server-fn prefix probe**: register `probed_server_fn_handler` from
   `crates/leptos-utils` on both `/api/{*fn_name}` and `/api/api/{*fn_name}`
   (Pattern 9).
10. **Graceful shutdown**: pick Pattern 15 (inline `main`) for single-task
    binaries, Pattern 17 (modular `bootstrap::run` / `shutdown::wait`) for
    ones with multiple long-lived tasks.
11. **Wiring**: add the new service to `docker-compose.yml` (pick a free
    port), `.woodpecker.yml` `leptosfmt` step's `find` predicate, and the
    `Cargo.toml` `[workspace.dependencies]` table if it needs workspace-shared
    crates.
12. **Verify**: `cargo check --workspace --all-targets`,
    `cargo clippy --workspace --all-targets -- -D warnings`,
    `cargo test --workspace --lib`, then `cargo leptos build` end-to-end.

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

### Bundle of Patterns (for AI model loading)

When you need deeper patterns, load the relevant reference file. Keep the
following quick links handy — they map human questions to the canonical
SKILL.md section.

- **Writing Leptos code?** → `references/leptos-patterns.md` (Patterns 1, 10, 11, 19, 21, 23 in SKILL.md)
- **Setting up PostgreSQL?** → `references/postgres-patterns.md` (Patterns 3, 5, 8, 22, 24)
- **Configuring axum routes?** → `references/axum-patterns.md` (Patterns 1, 2, 9, 14, 15, 17)
- **Writing tests?** → `references/testing-patterns.md` (Patterns 4, 12, 13, 15)
- **Designing architecture?** → `references/architecture-patterns.md` (Patterns 1, 2, 6, 17, 18)

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
        .max_connections(21) // 20 for queries + 1 for PgListener (see Critical Rule 3)
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

> **Note on graceful shutdown:** the snippet above does not wire SIGINT /
> SIGTERM into the server, so `cargo leptos build && ./live-search` will not
> finish in-flight requests on Ctrl+C. For the canonical `axum::serve` +
> `CancellationToken` + `JoinSet` triad (long-lived tasks, OTel flush on
> exit, drain timeout), see [Pattern 15](#pattern-15-structured-concurrency-triad-cancellationtoken-joinset-select)
> and [Pattern 17](#pattern-17-modular-server-lifecycle-bootstraprun--shutdownwait).
> For binaries with zero spawned tasks (the gateway, for example),
> `axum::serve(...).with_graceful_shutdown(signal_future)` is sufficient.
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

> The `pg_listener_task` below is a long-lived `tokio::spawn`'d task. For
> cancellation-safe structured concurrency (signal handler + `CancellationToken`
> + `biased;` shutdown semantics), see [Pattern 15](#pattern-15-structured-concurrency-triad-cancellationtoken-joinset-select).
> The canonical implementation lives in `live-search/src/db.rs::run_pg_listener`.

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
    // Surface broadcast lag as an explicit SSE event so slow consumers can
    // see they've fallen behind — silently dropping messages masks real
    // backpressure issues in production. Canonical full version with the
    // publisher + Event struct lives in references/axum-patterns.md §6.
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
        |result| async move {
            match result {
                Ok(event) => Some(Ok(event)),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "SSE client lagged; skipping old events");
                    Some(Ok(Event::default()
                        .event("stream_lagged")
                        .data(format!("lagged:{n}"))))
                }
                Err(broadcast::error::RecvError::Closed) => None,
            }
        },
    );
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

> **Canonical implementation extends this pattern.** The simple form above is
> the teaching prologue. The production `live-search/src/db.rs::run_pg_listener`
> adds:
> - Exponential backoff (`250 ms → 30 s`, doubling, reset on successful
>   connect/recv).
> - A separate `run_watchdog` task that increments a shared
>   `Arc<AtomicU64>` reconnect counter if no notification arrives within
>   `WATCHDOG_STALE_THRESHOLD` (90 s). The listener observes this counter
>   on each `recv()` cycle and reconnects even when the connection looks
>   healthy from the OS's perspective.
> - `biased;` in the inner `select!` so shutdown always wins ties against
>   an incoming NOTIFY.
> - Cancellation safety by racing `recv()` against
>   `shutdown.cancelled()` via `tokio::select!`, with `sleep_or_shutdown`
>   for backoff intervals.

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

#### Trigger that bridges INSERT → NOTIFY

```sql
-- From live-search/migrations/001_create_search_results.up.sql
CREATE OR REPLACE FUNCTION notify_search_result()
RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify(
        'search_results',
        json_build_object(
            'type', 'SearchResult',
            'id', NEW.id,
            'title', NEW.title,
            'created_at', NEW.created_at
        )::text
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_search_result_notify
    AFTER INSERT ON search_results
    FOR EACH ROW
    EXECUTE FUNCTION notify_search_result();
```

The pipeline diagram in Pattern 2 only works because of this trigger — without it, `INSERT` rows never reach the PgListener.

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
            // is already headless. `HeadlessMode` IS publicly exported at
            // `chromiumoxide::browser::HeadlessMode`, but is only needed for
            // non-default headless configurations.
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
        // Use eprintln! (not tracing::warn!) — no tracing subscriber is
        // initialised in E2E test binaries; see Pattern 12 for rationale.
        eprintln!("failed to remove Chromium profile dir: {e}");
    }
}
```

> **Lint compatibility:** the canonical workspace sets
> `clippy::unwrap_used = "deny"` and `clippy::expect_used = "deny"` for
> production crates. Test crates relax these (`unwrap_used = "allow"`,
> `expect_used = "allow"` in `e2e-tests/Cargo.toml`) so `.unwrap()` /
> `.expect()` for fail-fast test setup are acceptable there. Never copy
> these patterns into production code.

```rust
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

#### Local dev keypair (`--dev-keys`)

The `--dev-keys` flag generates an ephemeral EdDSA keypair for local
development, removing the need to configure JWKS or real signing keys.

- **Requires `ALLOW_DEV_KEYS=1`** env var (defence in depth — prevents
  accidental use in deployment)
- Generated keypair is held only in memory; only FNV-1a fingerprints are
  logged for diagnostic correlation
- **Never use in production** — the private key material is ephemeral and
  cannot be rotated
- See `gateway/src/settings.rs::Settings::load_dev_keys` for the
  implementation. The binary entrypoint calls
  `Settings::load_dev_keys_from_env` which reads `ADMIN_PASSWORD`
  from the environment and delegates to `load_dev_keys`.

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

### Scoped CSS with stylance

The workspace uses [`stylance`](https://crates.io/crates/stylance) for
scoped CSS without a build-step dependency:

```rust
// In your Leptos component file
stylance::import_style!(pub css, "styles.module.css");
// `css` is a namespace module with typed identifiers:
//   css::nav, css::container, css::search_input, etc.
// Re-export for callers:
pub use css::*;
```

Callers then write `styles::nav` instead of raw class name strings:

```rust
use crate::styles;

view! {
    <nav class=styles::nav>
        <input class=styles::search_input type="text" />
    </nav>
}
```

Key properties:
- **No `stylance-cli` build step needed** — the `stylance!` proc-macro
  runs at compile time, rewriting class names directly in the generated
  code
- CSS files live next to their component (`styles.module.css`), co-located
  with the Rust source
- See `live-search/src/styles.rs` for the workspace's canonical usage

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
// e2e-tests/src/common/chromium.rs — canonical helpers

use chromiumoxide::{Browser, BrowserConfig, Page};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub fn unique_profile_dir() -> std::path::PathBuf {
    // A pure per-process atomic counter is sufficient and safer than the
    // previously-shown `SystemTime::now()` approach: `SystemTime` is not
    // monotonic (NTP jumps, manual clock changes can repeat nanos values
    // across parallel tests) and there's no failure path to handle.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = format!("/tmp/chromiumoxide-{pid}-{n}", pid = std::process::id());
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

Verified stable on this host: Chromium **1208** (Playwright 1.50 era). If you
hit launch crashes on a newer Chromium build, pin via `CHROME_PATH` rather than
chasing the latest.
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
        // No `biased;` here: both branches are symmetric shutdown signals,
        // so whichever fires first cancels the token. (See Pattern 15's
        // `biased;` section for cases where branch priority matters.)
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

> **For binaries with multiple long-lived tasks** (e.g. a server + a PgListener + a background housekeeper), the ad-hoc `main`-body pattern above is sufficient for two tasks but gets unwieldy past three. See [Pattern 17](#pattern-17-modular-server-lifecycle-bootstraprun--shutdownwait) for the modular `bootstrap::run` + `shutdown::wait` split that scales to N tasks.

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

### Cross-cutting Concerns: CSRF, CSP, CORS

Three security headers and policies the gateway enforces by default:

- **CORS** (`gateway/src/cors.rs`): allowlist-driven via `ALLOWED_ORIGINS`
  env var. Default is the dev localhost allowlist (`:3000`, `:3001`, `:3002`).
  Special value `*` permits any origin (debug only — emits a `warn!`).
- **CSRF** is not provided by this workspace: there is no session-bearing
  cookie, so the Synchronizer Token Pattern is inapplicable. If you adopt
  session-cookie auth, see `axum-tower-sessions-csrf` and integrate it as a
  sibling layer to the existing CORS / CSP layers.
- **CSP** (`gateway/src/cors.rs::csp_layer`): Content-Security-Policy
  header set via `tower_http::set_header::SetResponseHeaderLayer`. The
  default policy allows self-hosted scripts/styles, `'unsafe-inline'` for
  Leptos SSR, and `ws:`/`wss:` for WebSocket/SSE connections.

Production checklist:
1. Override `ALLOWED_ORIGINS` to your actual domain(s).
2. Override `SESSION_COOKIE_SECURE=true` (the default is `true` already).
3. Verify CSP allows any required external script/style sources.
4. Don't use `ALLOWED_ORIGINS=*` outside local development.

### Pattern 17: Modular Server Lifecycle (`bootstrap::run` + `shutdown::wait`)

For binaries that spawn more than one long-lived task, routing the
full triad (server, PgListener, background housekeepers) through
`main()` quickly becomes boilerplate. This pattern extracts the
lifecycle into two functions — `bootstrap::run()` and `shutdown::wait()`
— that mirror the actual `bootstrap` / `shutdown` split in the
`live-search` crate. Code below is copied verbatim from
`live-search/src/bootstrap.rs` / `live-search/src/shutdown.rs`.

**`bootstrap::run()` (abridged, canonical source comments stripped)**:

```rust
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Handle returned by `run()`. The caller uses the `CancellationToken`
/// to signal shutdown and the `JoinSet` / `PgPool` for graceful draining.
#[derive(Debug)]
#[must_use]
pub struct ServerHandle {
    pub shutdown: CancellationToken,
    pub tasks: JoinSet<anyhow::Result<()>>,
    pub pool: sqlx::PgPool,
}

pub async fn run() -> anyhow::Result<ServerHandle> {
    // 1. Tracing subscriber (plain fmt or with OTel via `otel` feature).
    init_tracing();

    // 2. Workspace config (overridable via RWF_* env vars and
    //    config.toml — see Pattern 18).
    let cfg = rwf_config::Config::load()?;
    let database_url = std::env::var("DATABASE_URL")
        .ok()
        .unwrap_or_else(|| cfg.live_search.database_url.clone());

    // 3. Pool with rwf-config tunables.
    let pool_tunables = db::PoolTunables {
        max_connections: cfg.live_search.pool_max_connections,
        min_connections: cfg.live_search.pool_min_connections,
        acquire_timeout_secs: cfg.live_search.pool_acquire_timeout_secs,
        idle_timeout_secs: cfg.live_search.pool_idle_timeout_secs,
        max_lifetime_secs: cfg.live_search.pool_max_lifetime_secs,
    };
    let pool = db::create_pool(&database_url, &pool_tunables).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    // 4. Cache + broadcast channel + AppContext. Round 5f replaced
    //    the previous `cache::init_cache()` / `db::set_pool(...)` /
    //    `sse::set_broadcast(...)` OnceLock pattern with a single
    //    `AppContext` plumbed through `state::set()` (server fns)
    //    and `leptos::provide_context()` (SSR component tree).
    let cache_handle = CacheHandle::default();
    let (tx, _rx) =
        tokio::sync::broadcast::channel::<SseEvent>(cfg.live_search.sse_broadcast_buffer);
    let ctx = Arc::new(state::AppContext::new(pool.clone(), tx.clone(), cache_handle.clone()));
    state::set(Arc::clone(&ctx))?;

    // 5. Cancellation token + JoinSet.
    let shutdown = CancellationToken::new();
    let mut tasks = JoinSet::new();

    // 6. Spawn long-lived tasks with child tokens (PG listener, watchdog,
    //    HTTP server, etc. — see source for full list).
    let listener_token = shutdown.child_token();
    let watchdog_token = shutdown.child_token();
    // ...tasks.spawn(...).instrument(span)...

    Ok(ServerHandle { shutdown, tasks, pool })
}
```

**`shutdown::wait()` (abridged)**:

```rust
pub async fn wait(
    shutdown: CancellationToken,
    tasks: &mut JoinSet<anyhow::Result<()>>,
    pool: &sqlx::PgPool,
) -> anyhow::Result<()> {
    // 1. Install the Ctrl+C / SIGTERM handler. Closes the gap between
    //    "signal arrived" and "shutdown token cancelled".
    let signal_token = shutdown.clone();
    tokio::spawn(async move {
        let ctrl_c = async { signal::ctrl_c().await.expect(...) };
        #[cfg(unix)] let terminate = /* SIGTERM handler */;
        let _ = /* tokio::select! { ctrl_c, terminate } */;
        signal_token.cancel();
    });

    // 2. Block until the token fires.
    shutdown.cancelled().await;

    // 3. Close the DB pool with a 5 s timeout (PgListener's borrowed
    //    connection is force-released on `pool.close()`).
    db::close_pool(pool).await;

    // 4. Drain the JoinSet with a 10 s timeout; abort on timeout.
    shutdown.cancel();
    match tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!(error = %e, "task error"),
                Err(join_err) if join_err.is_panic() =>
                    tracing::error!(error = ?join_err, "task panicked"),
                Err(join_err) =>
                    tracing::warn!(error = ?join_err, "task did not complete cleanly"),
            }
        }
    }).await {
        Ok(()) => {}
        Err(_elapsed) => tasks.abort_all(),
    }

    // 5. Force-flush any OTel providers (5 s timeout each) so telemetry
    //    isn't lost on exit.
    #[cfg(feature = "otel")]
    if let Some(provider) = crate::bootstrap::get_tracer_provider() {
        let provider = provider.clone();
        let _ = tokio::time::timeout(Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let _ = provider.force_flush();
                let _ = provider.shutdown();
            })).await;
    }

    Ok(())
}
```

**Usage in `main()`**:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut handle = bootstrap::run().await?;
    shutdown::wait(handle.shutdown, &mut handle.tasks, &handle.pool).await
}
```

**When to use this pattern**:

- Binary has more than one long-lived task (`tokio::spawn` or
  `JoinSet::spawn`). The gateway, which has zero spawned tasks, does
  **not** need this — `with_graceful_shutdown(signal_future)` is
  sufficient.
- You want a single call site for OTel shutdown so telemetry is never
  lost on exit.
- You are writing tests that need to start and stop the full server
  lifecycle without a process restart (call `handle.shutdown.cancel()`
  then `shutdown::wait(...)` to tear down).

**Cross-link to Pattern 15**: Pattern 15 covers the raw structured
concurrency triad (`CancellationToken` + `JoinSet` + `select!`) for
one-off binaries. Pattern 17 (this pattern) wraps that same triad in a reusable
`bootstrap`/`shutdown` contract; for binaries with a single task,
Pattern 15's in-line `main` body is simpler.

Reference implementations: `live-search/src/bootstrap.rs`,
`live-search/src/shutdown.rs`.

### Pattern 18: Layered config (TOML file + RWF_* env overrides)

All workspace binaries read their configuration through the
`rwf-config` crate (`crates/config/`). This avoids per-binary
ad-hoc `std::env::var` chains and the hand-rolled merge logic that
sometimes tempted people toward `unsafe` to satisfy the borrow checker.

**Resolution order** (highest priority wins):

1. `RWF_*` environment variables — e.g. `RWF_GATEWAY__PORT=4000`
2. `config.toml` at workspace root (or `RWF_CONFIG=/path/to/config.toml`)
3. Defaults baked into the `Config` structs

The `__` separator addresses nested keys (`gateway.cors.allowed_origins`
becomes `RWF_GATEWAY__CORS__ALLOWED_ORIGINS`).

**Usage in a binary:**

```rust
use rwf_config::Config;

let cfg = Config::load().context("failed to load workspace config")?;
let port = std::env::var("PORT")
    .ok()
    .and_then(|p| p.parse().ok())
    .unwrap_or(cfg.gateway.port);
```

**Reference `config.toml`:**

```toml
[gateway]
port = 3001
proxy_upstream_url = "https://ipapi.co"

[gateway.cors]
allowed_origins = "http://localhost:3000,http://localhost:3001,http://localhost:3002"

[gateway.session]
cookie_secure = true

[live_search]
port = 3000
database_url = "postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo"

[otel]
endpoint = "http://127.0.0.1:4317"
```

A complete, documented example is in `config.toml.example` at the workspace
root — copy it to `config.toml` to activate.

When adding a new setting:

1. Add the field to the appropriate section struct in
   `crates/config/src/lib.rs` (with a `Default` impl if it should be
   optional).
2. Add `set_default(...)` for it in `Config::load()` so the loader has
   a fallback.
3. Add the key to `config.toml` at the workspace root so contributors
   can see it.
4. Add a `#[expect(clippy::derivable_impls)]`-free default — the
   pedantic lint will catch it.

`Config::load()` returns a `Result<Config, ConfigError>` with two
variants: `Load(config::ConfigError)` for parser/IO failures and
`ConfigPathNotFound(String)` when `RWF_CONFIG` was set but the file
doesn't exist. **Never silently fall back** — both variants propagate
the failure to the user so misconfigurations are immediately obvious.

### Pattern 19: Leptos 0.8.x Knowledge Patch

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

`<ShowLet>` is a single-bind shorthand that accepts an `Option<T>` signal via the `some` prop and destructures it for children:

```rust
// Equivalent:
<Show when=move || user.get()>
    {move || user.get().map(|u| view! { <p>{u.name.to_string()}</p> })}
</Show>

<ShowLet
    some=move || user.get()    // Signal<Option<T>> in 0.8.8+
    let:value
>
    <p>{value.name.to_string()}</p>
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
entry is needed** for `bitcode` itself** — `leptos::server_fn::codec::Bitcode`
is re-exported by the `server_fn = "0.8"` crate (`pub use bitcode;` in
`server_fn-0.8.13/src/lib.rs`). The codec is therefore available wherever
`leptos` is a dependency. If you want to call `bitcode` APIs directly
outside of `#[server]`, add `bitcode = "0.6"` explicitly.

### Pattern 20: Leptos Utility Ecosystem

The canonical workspace implements the Leptos 0.8 core (`leptos`,
`leptos_axum`, `leptos_router`, `leptos_meta`) plus one ecosystem crate
(`leptos_i18n`). The wider [`awesome-leptos`](https://github.com/leptos-rs/awesome-leptos)
list contains dozens more. Below is the curated shortlist the skill
endorses (or warns against). Add new ones only after you have read the
caveat column.

| Crate | Role | Tier | Status in this skill | Caveat |
|-------|------|------|---------------------|--------|
| [`leptos_i18n`](https://crates.io/crates/leptos_i18n) (0.6) | Compile-time-checked translations, JSON locales, `t!` / `t_string!` macros, runtime `set_locale` | Tier 1 | **Used.** Workspace `i18n-demo` crate shows the full pattern (5 keys, EN + DE, locale switcher). | Requires `leptos_i18n_build` build-dep and a `locales/*.json` tree; build.rs runs `Config::new("en")?.add_locale("de")?` and emits the typed module. |
| [`leptosfmt`](https://crates.io/crates/leptosfmt) | `view!` macro formatter; chains with `rustfmt --rustfmt` | Tier 1 | **Used.** `.woodpecker.yml` runs `leptosfmt --check .` after `cargo fmt --check`. | `cargo install leptosfmt --locked`. Editor integration via `rust-analyzer.toml`'s `overrideCommand`. |
| `gloo-net` (0.7) | Client-side EventSource + reconnect loop | Tier 1 | **Used.** `live-search::LiveFeedPage` consumes `/api/events` via `gloo_net::eventsource::futures::EventSource` with a 2 s reconnect; preserves the `Arc<str>` payload shape. | Hand-rolled cleanup signal via `RwSignal<bool>` + `on_cleanup` because the WASM target has no automatic cancellation. |
| [`leptos-use`](https://leptos-use.rs/) (~90 hooks) | `use_event_source`, `use_cookie`, `use_debounce_fn`, `use_intersection_observer`, `use_media_query`, etc. | Tier 1 | **Used.** `live-search/src/app.rs:22` imports `use leptos_use::watch_debounced;` for the debounced search box. | Two `leptos-use` versions coexist in `Cargo.lock` (0.18.3 transitive from `leptos_i18n 0.6.2`, 0.19.0 direct). Neither pulls `getrandom`. The `getrandom 0.3.4` you may see in `cargo tree` comes from `governor 0.10.4` (via `tower_governor 0.8`), and is benign on Linux native + wasm32 because `governor` is only compiled for the SSR binary. |
| [`leptos_sse`](https://github.com/messense/leptos_sse) | Server-pushed reactive signals over SSE, with JSON-patch sync | Tier 2 | **Not used.** | Different pattern than Pattern 2: `leptos_sse` is for "the server holds state and the client gets a mirror signal", **not** for raw client-side SSE consumption. Use `gloo-net::EventSource` for raw event protocols; use `leptos_sse::create_sse_signal` when you want automatic JSON-patch state replication. |
| [`tailwind-fuse`](https://github.com/gaucho-labs/tailwind-fuse) | `tw_merge!` / `tw_join!` plus `#[derive(TwClass)]` / `#[derive(TwVariant)]` macros | Tier 1 | **Not used.** | Skill projects use inline `style="…"` attrs throughout. `tailwind-fuse` is the recommended merge-and-variant helper if/when you adopt Tailwind — install with `cargo add tailwind-fuse --features variant` once your `class="…"` strings appear. |
| [`leptos-declarative`](https://github.com/jquesada2016/leptos-declarative) | Auto-generates `#[component]` prop structs from inner struct fields | Tier 2 | Mentioned as a "fewer boilerplate for many components" footnote next to component-heavy projects. | Use when components outgrow manual `(signal, set_signal)` plumbing; not in this skill since the demo crates are small. |
| [`leptos_oidc`](https://gitlab.com/kerkmann/leptos_oidc) | OIDC integration (Keycloak / Auth0 / etc.) | Tier 2 | Mentioned as a "swap JWT for OIDC" footnote inside `gateway`'s `auth.rs`. | `gateway-example` deliberately uses a hand-rolled `jsonwebtoken` middleware for full control over claims. Replace with `leptos_oidc` when integrating an existing IdP. |
| [`leptos_ws`](https://github.com/TimTom2016/leptos_ws) | Leptos signal ↔ WebSocket bridge | Tier 2 | Mentioned as "for bidirectional real-time" footnote next to Pattern 2. | The skill covers SSE (server→client push) thoroughly; reach for `leptos_ws` when clients also push state to the server (chat, collaborative editing). |
| [`leptos-material`](https://github.com/jordi-star/leptos-material) | Material Web Components wrapped for Leptos | Tier 2 | Mentioned as one of several ready-made UI kit options. | Skill focuses on backend / signal patterns, not design polish. Use `leptos-material` (or [`thaw`](https://github.com/thaw-ui/thaw), [`leptix`](https://github.com/leptix/leptix), [`Rust shadcn/ui`](https://shadcn-ui.rustforweb.org), [`Rust/UI`](https://github.com/rust-ui/ui), [`leptos-struct-table`](https://github.com/Synphonyte/leptos-struct-table)) when you need a design system on top of these patterns. |
| [`leptos-fetch`](https://github.com/zakstucke/leptos-fetch) | Async data fetching cache | Tier 2 | Optional footnote. | Skill's `Action::value()` + `Resource` covers the same need; reach for `leptos-fetch` if you want a React-Query-equivalent API. |
| [`leptos-image`](https://github.com/gaucho-labs/leptos-image) | WebP image optimizer + LQIP for SSR | Tier 3 | Not referenced. | Useful only for image-heavy apps. |
| `leptos-mview` (alternate `view!` macro) | Maud-style concise `view!` | Tier 3 | Not used. | Conflicts with the skill's "always use Leptos's `view!`" rule. |

When adopting any ecosystem crate, follow Critical Rules 1, 4, 6, and
Pitfall 15 — feature-flag gating, config injection, server-fn prefix
doubling, and `#[server]` body cfg-gating are the same whether the
function lives in `live-search`, `i18n-demo`, or a brand-new crate.

### Pattern 21: ErrorBoundary + ActionForm (Progressive Enhancement)

Two complementary Leptos 0.8 patterns that make UI more robust:

**ErrorBoundary** wraps a subtree and renders a fallback view when the subtree
panics during rendering or a server function returns an unrecoverable error.
Critical for production apps — without it, a single bad row in a list crashes
the whole page.

**ActionForm** is a `<form>` component that submits directly to a Leptos
`Action` (server fn) via standard HTTP POST. The form works WITHOUT JavaScript
(progressive enhancement) — even if WASM hydration fails, the form submits
normally and the server fn handles the request.

**ErrorBoundary example**: see `live-search/src/app.rs::SearchErrorBoundary`
(wraps the result list, displays a recovery notice on panic).

**ActionForm example**: see `live-search/src/app.rs::SearchPage` (uses a manual
`<form on:submit>` because the action is also driven by a debounced
`watch_debounced`; a pure `ActionForm` example is documented as a pattern option
but not exercised in the live-search showcase because `ActionForm` requires
`ServerAction<_>`, not `Action<_, _>`).

When to use:
- **ErrorBoundary**: every component that renders server-function results,
  every `<For>` loop over data that might be malformed
- **ActionForm**: any form where users without JS should still be able to
  submit (public-facing sites, SEO-critical pages)

### Pattern 22: Cursor-Based Pagination

For result sets that may grow unbounded, never `LIMIT 20` and hope — use
cursor-based pagination so clients can iterate without offset drift.

**Server side**: the search query takes an optional cursor `(created_at, id)`
and uses row-value comparison `(created_at, id) < ($2, $3)` in the WHERE
clause. ORDER BY `created_at DESC, id DESC` for a stable scan.

```rust
sqlx::query_as::<_, SearchResult>(
    r"SELECT id, title, url, snippet, created_at
       FROM search_results
       WHERE fts @@ plainto_tsquery('english', $1)
         AND (created_at, id) < ($2, $3)
       ORDER BY created_at DESC, id DESC
       LIMIT $4",
)
.bind(query)
.bind(cursor_time)
.bind(cursor_id)
.bind(limit)
.fetch_all(pool)
.await
```

**Cursor format**: encode the last row's `(created_at, id)` as a base64url
string of `"{timestamp_micros}|{uuid}"`. Clients pass it back as the
`cursor` argument on the next request.

```rust
pub fn encode_cursor(created_at: DateTime<Utc>, id: Uuid) -> String;
pub fn decode_cursor(s: &str) -> Result<(DateTime<Utc>, Uuid), String>;
```

Canonical implementation: `live-search/src/db.rs::search_with_cursor` and
`live-search/src/db.rs::{encode_cursor, decode_cursor}`.

When to use cursor vs offset:
- **Cursor**: real-time data feeds, infinite scroll, large result sets
- **Offset**: admin dashboards, paginated tables where users jump to page N

### Pattern 23: Leptos Islands Architecture

Leptos 0.8 supports an "islands" architecture where the server renders
mostly static HTML and selectively hydrates small interactive regions.
This is the inverse of full SSR+hydration — most of the page is plain HTML,
and only the interactive parts become WASM.

Use `#[island]` (instead of `#[component]`) for components that should
hydrate on the client. The server still renders them as HTML, but only
islands get the `data-island` attribute and are scheduled for hydration.

```rust
use leptos::prelude::*;

#[island]
pub fn Counter() -> impl IntoView {
    let (count, set_count) = signal(0);
    view! {
        <button on:click=move |_| set_count.update(|n| *n += 1)>
            "Count: " {count}
        </button>
    }
}
```

> **Feature gate**: `#[island]` requires `leptos/experimental-islands` in
> `Cargo.toml` features. Leptos 0.8.20+ exposes this feature; earlier
> versions will fail to compile.

Server-side rendering of an island produces the HTML; the client
hydrates only the island, not the entire page. This is the right
architecture for:
- Content-heavy sites (blogs, docs, marketing pages)
- Pages where 95% of the content is static and 5% is interactive
- Reducing WASM bundle size and initial JS execution time

When NOT to use islands:
- Highly interactive apps (dashboards, editors) — full hydration is fine
- Pages with heavy client-side state — islands add complexity for state
  shared across components

Live-search and i18n-demo do not currently use islands; the showcase
demonstrates full hydration. Adopt islands when migrating to a
content-heavy site.

### Pattern 24: Atomic Refresh-Token Rotation (PostgreSQL)

Access JWTs are short-lived; refresh tokens outlive them. The
gateway stores refresh tokens as SHA-256 hashes (never plaintext) and
rotates them atomically inside a single transaction.

**Schema** (`gateway/migrations/100_create_refresh_tokens.up.sql`):

```sql
CREATE TABLE refresh_tokens (
    jti          UUID        PRIMARY KEY,
    subject      UUID        NOT NULL,
    hashed_token BYTEA       NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    revoked_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_refresh_tokens_hashed_token ON refresh_tokens (hashed_token);
CREATE INDEX idx_refresh_tokens_subject     ON refresh_tokens (subject);
CREATE INDEX idx_refresh_tokens_expires_at  ON refresh_tokens (expires_at);
```

**Rotation semantics** (`gateway/src/auth/refresh.rs::rotate`):

1. Begin `tx`.
2. `SELECT … WHERE hashed_token = $1 AND revoked_at IS NULL AND expires_at > NOW() FOR UPDATE`.
   The `FOR UPDATE` row-lock prevents two concurrent rotations of the
   same token from racing.
3. If the row exists, `UPDATE … SET revoked_at = NOW()` and insert a
   fresh token row with the same subject.
4. Commit.
5. Re-using an already-rotated token returns `Ok(None)` → handler
   maps to `401`. **Treat this as a stolen-credential signal**, not
   a normal 4xx. Production code should additionally email the user.

**Refresh handler dual behaviour** (`gateway/src/auth/handlers.rs`):

| `GatewayState.db_pool` | Request body | Behaviour |
| ---------------------- | ------------ | --------- |
| `Some(pool)`           | `{"refresh_token": "..."}` | DB-backed rotation, returns `{token, refresh_token: "..."}` |
| `None`                 | `{"token": "..."}` | Legacy stateless refresh (re-issue new JWT for same subject) |

The legacy path lets the example run without `PostgreSQL` configured
while still exercising the production-grade rotation flow when
`DATABASE_URL` is set.

**Test or it didn't happen** — every `tx` is `(FOR UPDATE + revoke + insert)`
in one transaction. Without the row lock, a stolen token can be
rotated twice in parallel and the second rotation wins, leaving the
attacker's token valid while the legitimate user's next refresh
fails — bad UX *and* a credential-theft oracle.

### Pattern 25: WebSocket Chat via static broadcast hub

`i18n-demo/src/ws_chat.rs` exposes `/ws/chat` — a bidirectional
chat endpoint backed by a single static `tokio::sync::broadcast::Sender`.
The pattern is deliberately minimal so the deployment shape is easy
to recognise before swapping in a real pub/sub backend.

> This demo uses a single static `broadcast::Sender` and does not spawn
> background tasks, so it does NOT need `CancellationToken` wiring. If you
> extend the hub with a separate publisher task, follow
> [Pattern 15](#pattern-15-structured-concurrency-triad-cancellationtoken-joinset-select).

**Server side:**

```rust
static HUB: LazyLock<broadcast::Sender<ChatEvent>> =
    LazyLock::new(|| broadcast::channel(256).0);

pub fn chat_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let from = Uuid::new_v4();
    let mut rx = HUB.subscribe();
    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            incoming = stream.next() => { /* client -> hub */ }
            broadcast = rx.recv()   => { /* hub -> client */ }
        }
    }
}
```

**Client side** (any browser):

```js
const ws = new WebSocket("ws://localhost:3002/ws/chat");
ws.onmessage = (e) => console.log(JSON.parse(e.data));
ws.onopen = () => ws.send("hello");
```

**Hardening checklist (swap in before production):**

1. Replace the per-connection random `Uuid` with the authenticated
   session id (the gateway's `Claims` struct). Don't ship chat that
   lets anyone claim any name.
2. Move the `HUB` static behind `tokio::sync::RwLock` or onto
   Redis pub/sub once you have more than one node —
   `broadcast::Sender` is a single-process construct.
3. Add a backpressure policy: cap per-frame text at 1 KiB (already
   enforced in the demo) and reject slow clients at the edge.
4. Add a periodic ping/pong so dead-but-connected sockets are
   reaped. `axum::extract::ws::Message::Ping` is already a no-op in
   the current handler.
5. Track the subscription count via `HUB.receiver_count()` and
   expose it from `/metrics`.

**Tests:**

- `ws_chat::tests::max_text_bytes_matches_documented_value`
- `ws_chat::tests::new_event_has_unique_ids_and_timestamps`
- `ws_chat::tests::event_round_trips_through_serde_json`

---

## Common Pitfalls

### 1. PgListener connection leak
Always call `listener.listen()` BEFORE entering the recv loop; the connection is held for the listener's lifetime.

### 2. Broadcast channel overflow
Default buffer is 256; lagging consumers should receive an explicit `stream_lagged`/diagnostic event and publishers should log `SendError` when no receivers exist.

### 3. Leptos SSR hangs
If a server function never resolves, the SSR stream blocks indefinitely — use `.timeout()` on async operations.

### 4. JSONB in sqlx macros
Use `as _` cast for Json<T> in `query!()` macros; otherwise the macro can't infer the type. (The cast is needed for **type inference**, not to "skip type verification" — the column type is still checked.)

### 5. Feature flag conflicts
`csr`, `ssr`, `hydrate` are mutually exclusive — use `[features]` section in Cargo.toml to enforce this with `skip_feature_sets`.

### 6. cross-origin SSE
EventSource requires same-origin by default; use CORS headers or serve from same domain.

### 7. chromiumoxide user_data_dir collision
Default `~/.cache/chromiumoxide-runner/SingletonLock` collides when tests run in parallel. Always set a unique `user_data_dir` per test (see Pattern 12).

### 8. WASM hydration requires static serving
SSR HTML references `/pkg/{crate}.js` and `/pkg/{crate}_bg.wasm`. Without `ServeDir::new("./pkg")` mounted on the router, the page renders but JavaScript never runs. Verify with `curl http://localhost:3000/pkg/live_search.js` returning 200.

### 9. Server-fn 404 / doubled-prefix
`endpoint = "/api/search"` + `handle_server_fns` mounted at `/api/*fn_name` → server fn is reachable only at `/api/api/search`. Either mount the route at `/api/api/*fn_name`, or use a catch-all handler that tries both (Pattern 9).

### 10. jsonwebtoken 10 panics without crypto provider
`jsonwebtoken = "10"` alone crashes the process on first `encode()`/`decode()` call with `Could not automatically determine the process-level CryptoProvider`. Fix with `features = ["rust_crypto"]` (pure Rust) or `["aws_lc_rs"]` (requires cmake/perl/nasm C toolchain). The workspace uses `aws_lc_rs`; switch to `rust_crypto` if your CI image lacks the C toolchain.

### 11. Silent test skips via `check_server_or_skip()`
Do not use helpers that return `false` and let tests `return`. Required dependencies should panic/assert with the actual status or error; optional slow tests should use `#[ignore]`.

### 12. Stale `target/debug/deps/` fingerprints
Every Cargo.toml change creates new `.rlib` hashes (e.g. `libplaywright-*.rlib`, `libchromiumoxide-{hash}.rlib`). Cargo never garbage-collects. Use `cargo clean` periodically or `cargo-sweep` to reclaim disk. Sccache eliminates compile time but does NOT shrink `target/`.

### 13. `sccache` is local-disk by default
No `SCCACHE_*` env vars or `~/.config/sccache/config.toml` means `~/.cache/sccache` (local). For remote/distributed caching, set `SCCACHE_BUCKET` (S3) or `SCCACHE_REDIS` (Redis) explicitly.

### 14. Background tasks missing CancellationToken wiring
`tokio::spawn(pg_listener_task(pool, tx))` without a `CancellationToken` parameter cannot be cancelled — the task runs forever even when the server is shutting down. The `recv().await` future IS cancel-safe (sqlx 0.9 PgListener drops the TCP read cleanly), so wrap the loop in `tokio::select!` against `token.cancelled()`, then fire the token from a Ctrl+C/SIGTERM handler in `main()`. See Pattern 15.

### 15. `#[server]` body not cfg-gated on `feature = "ssr"`
If the function body uses items gated by `#[cfg(feature = "ssr")]` (e.g. `crate::db::get_pool`, or a `sqlx::FromRow` derive that lives behind `#[cfg_attr(feature = "ssr", derive(sqlx::FromRow))]`) without an *explicit* `#[cfg(feature = "ssr")]` block on the body, then `cargo check --workspace --all-targets` (which compiles `live-search`'s lib with no features active) fails with `unresolved import crate::db::get_pool` / `the trait bound SearchResult: FromRow is not satisfied`. The `#[server]` macro does **not** auto-gate the body — gate it yourself, and add `#[allow(clippy::unused_async, reason = "...the non-ssr branch is a sync error stub...")]` so the empty non-ssr body stays compliant with `clippy::pedantic = "deny"`. Canonical fix in `./live-search/src/app.rs::search`.

> **TODO (deferred from council review):**
> - **Test cleanup on panic**: Pattern 4 / Pattern 12 in this skill use `std::fs::remove_dir_all` in test teardown. If the test panics before the cleanup block, the profile dir leaks. Migrate to `tempfile::TempDir` for panic-safe cleanup.
> - **`replace()` over `serde_json::json!()` for SSE JSON**: Pattern 14 shows `format!` + `{{ }}` escaping and `replace()` as primary. For complex JSON, `serde_json::json!()` is safer and easier to read. Reverse the recommendation.
> See `.slim/deepwork/skill-review-and-upgrades.md` for the full review history.

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
