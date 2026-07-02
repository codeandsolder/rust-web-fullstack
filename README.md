# rust-web-fullstack

A working showcase of **Leptos 0.8 + PostgreSQL + Axum** patterns from the `rust-web-fullstack` skill.
Four example crates demonstrating full-stack Rust patterns with server-side rendering,
hydration assets, real-time database notifications, service-oriented routing,
compile-time-checked internationalization, and end-to-end testing.

## What It Demonstrates

| Crate | Pattern | Highlights |
|-------|---------|------------|
| **live-search** | Full-stack Leptos SSR + PostgreSQL FTS | PgListener + SSE push, sqlx migrations, full-text search with `tsvector`, reactive UI |
| **gateway** | ServiceModule trait + multi-service composition | JWT auth middleware, Tower service layers, mock modular services |
| **i18n-demo** | Compile-time-checked internationalization | `leptos_i18n` 0.6 with JSON locales, `t!` macro, runtime locale switcher (EN ↔ DE) |
| **e2e-tests** | chromiumoxide E2E tests | Browser automation, SSE stream validation, cross-service integration |

## Architecture

```
                           ┌─────────────────┐
                           │    Browser       │
                           │ (Leptos WASM)    │
                           └──────┬──────────┘
                                  │ HTTP / SSE
                          ┌───────┴──────────┐
                          │   Axum Router     │
                          │   (:3000)         │
                          │   live-search     │
                          └───┬───────────────┘
                              │ sqlx
                      ┌───────┴──────────┐
                      │   PostgreSQL 17   │
                      │   FTS + NOTIFY    │
                      └───────┬──────────┘
                              │ PgListener
                      ┌───────┴──────────┐
                      │   SSE Stream      │
                      │   (tokio + tower) │
                      └──────────────────┘

┌──────────────────┐    ┌──────────────────┐
│   Gateway (:3001)│    │  i18n-demo (:3002)│
│   JWT Auth       │    │  leptos_i18n 0.6  │
│   Mock services  │    │  EN ↔ DE switcher │
│   ServiceModule  │    │  t! macro         │
└──────────────────┘    └──────────────────┘
```

`i18n-demo` runs on port **3002** and has no DB dependency — the four
crates are independent builds composed in `docker-compose.yml`.

## Quick Start

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) + [Docker Compose](https://docs.docker.com/compose/install/)
- [Rust](https://rustup.rs/) 1.94 or later
- [psql](https://www.postgresql.org/download/) (for manual seeding)
- Chromium or Chrome for browser E2E tests

### Docker Compose (recommended)

```bash
# Clone the repo
git clone https://git.onhir.eu/jan/rust-web-fullstack.git
cd rust-web-fullstack

# Start everything
docker compose up --build -d

# Seed the database with sample data
./scripts/seed-db.sh

# Check logs
docker compose logs -f

# Visit
#   Live search: http://localhost:3000
#   Gateway:     http://localhost:3001/health
```

### Manual Setup

```bash
# 1. Start PostgreSQL
docker compose up -d postgres

# 2. Seed the database
./scripts/seed-db.sh

# 3. Run live-search (SSR mode)
DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  cargo run -p live-search --features ssr

# 4. In another terminal, run the gateway
cargo run -p gateway-example

# 5. Open http://localhost:3000
```

## Tests

```bash
# Unit tests
make test

# E2E tests. The script starts PostgreSQL and both services, then runs
# integration-feature browser/API tests against the correct BASE_URL.
make test-e2e

# Or manually:
./scripts/test-e2e.sh
```

## Feature Flags

The workspace defines **per-crate** Cargo feature flags that opt in to observability and developer tooling:

| Flag | Crates | Description |
|------|--------|-------------|
| `otel` | gateway, live-search | OpenTelemetry OTLP export via OTLP/HTTP+protobuf (reqwest+rustls). Off by default; requires a collector endpoint (`OTEL_EXPORTER_OTLP_ENDPOINT`). Wires `tracing-opentelemetry` layers, `axum-tracing-opentelemetry` middleware, and `sqlx-otel` pool instrumentation. |
| `dev-tools` | gateway, live-search | Development-only instrumentation: `console-subscriber` (Tokio console, requires `RUSTFLAGS="--cfg tokio_unstable"`), extra logging spans. Not for production. |

## EdDSA JWT Keys

The gateway uses **EdDSA (Ed25519)** for JWT signing — the modern, fast, small-signature algorithm that supersedes HS256. The crypto backend is `aws-lc-rs` via `jsonwebtoken`'s `aws_lc_rs` feature, which is the 2026 canonical choice for EdDSA in Rust. Each running gateway needs a key pair.

Generate a fresh key pair with OpenSSL:

```bash
# Private key (PKCS#8 PEM)
openssl genpkey -algorithm ed25519 -out jwt-private.pem

# Public key (SPKI PEM, derived from the private key)
openssl pkey -in jwt-private.pem -pubout -out jwt-public.pem
```

Point the gateway at the key files:

```bash
export JWT_PRIVATE_KEY_PEM="$(cat jwt-private.pem)"
export JWT_PUBLIC_KEY_PEM="$(cat jwt-public.pem)"
```

Both environment variables must be set; `Settings::load` rejects empty or non-PEM values with a descriptive error.

**`--dev-keys` fallback**: for local exploration only, start the gateway binary with `ALLOW_DEV_KEYS=1 --dev-keys` (both required). The guard exists to prevent accidentally booting the gateway with ephemeral keys in production. When enabled, the startup logs FNV-1a fingerprints of the generated keys (not the keys themselves); the private key never leaves process memory or logs. Keys vanish on restart and clients cannot verify old tokens.

**Security note**: never commit private keys. `.gitignore` already excludes `.env`; keep it that way. CI uses a fixed test pair under `scripts/test-keys/` that is **not** suitable for any non-test deployment.

## Benchmarks

Criterion benchmarks ship with the live-search and gateway crates:

- `cargo bench -p live-search` — search-result FTS query throughput.
- `cargo bench -p gateway-example` — JWT sign/verify throughput.

Run either with the criterion HTML reports enabled (`features = ["html_reports"]` is workspace-default).

## What's New (v3 Showcase Upgrade)

This version brings the project to a high-end Rust 2026 showcase standard:

- **EdDSA JWT** with `aws-lc-rs` — replaces HS256. Key pair loaded from `JWT_PRIVATE_KEY_PEM` / `JWT_PUBLIC_KEY_PEM` env vars with a `--dev-keys` fallback.
- **Refresh token scaffolding** — `refresh_tokens` migration ships (jti UUID PK, subject, hashed token, expiry, revocation tracking) but the gateway handler wiring is TODO. The current `refresh_handler` / `logout_handler` are documented stubs. Wire them up via a follow-up PR.
- **OpenTelemetry** — OTLP export behind the `otel` feature flag. Wires `tracing-opentelemetry`, `axum-tracing-opentelemetry`, `sqlx-otel` for end-to-end trace propagation.
- **axum-prometheus** — `/metrics` endpoints on gateway and live-search.
- **tower-governor** — rate limiting on gateway auth routes (two governor instances for login vs. refresh).
- **tower-sessions + CSRF** — HttpOnly/Secure/SameSite=Lax session cookies with `axum-tower-sessions-csrf`.
- **axum-valid + validator** — typed, validated DTOs on every public gateway handler.
- **Trigram fuzzy search** — `pg_trgm` GIN index on `search_results.title` for typo-tolerant search.
- **pg_stat_statements** — query performance monitoring, preloaded via Postgres config.
- **moka cache** — hot search query caching in live-search (60s TTL, 1000 entries).
- **stylance** — scoped CSS replaces inline `style=` in all frontend crates.
- **leptos-use**, **leptos-struct-table**, **lepticons**, **leptos-forms-rs** — richer Leptos ecosystem integration.
- **utoipa + Swagger UI** — documented API at `/docs` on the gateway.
- **criterion benchmarks** — throughput bench for JWT auth, latency bench for search queries.
- **console-subscriber** — Tokio console instrumentation behind `dev-tools` feature.
- **testcontainers** — per-test Postgres 17 isolation in E2E tests, removing the shared-db dependency.
- **Nextest, squawk, sccache** — CI tooling upgrades for faster runs and SQL migration linting.

## CI/CD

This project uses [Woodpecker CI](https://woodpecker-ci.org/) with Forgejo Actions.
The `.woodpecker.yml` workflow runs:

| Step | Description |
|------|-------------|
| `check-workspace` | Workspace, SSR, and hydrate/WASM `cargo check` |
| `unit-tests` | `cargo test --workspace --lib` |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `clippy-live-search-ssr` | `cargo clippy -p live-search --features ssr --all-targets -- -D warnings` |
| `fmt` | `cargo fmt --all -- --check` |
| `e2e-tests` | Full E2E suite against PostgreSQL 17 |

## Project Structure

```
rust-web-fullstack/
├── Cargo.toml                  # Workspace root (edition 2024, lints table)
├── rust-toolchain.toml         # Pinned Rust 1.94.0 + WASM target
├── Cargo.lock
├── clippy.toml                 # Strict clippy lints for crates & examples
├── docker-compose.yml          # Postgres + live-search + i18n-demo + gateway
├── Makefile                    # Common development commands
├── ci/
│   └── leptosfmt.Dockerfile    # CI image with leptosfmt preinstalled
├── .woodpecker.yml             # CI pipeline (check, clippy, fmt, audit, e2e)
├── .env.example                # Environment variable template
├── .dockerignore               # Docker build exclusions
├── LICENSE                     # MIT
├── README.md                   # ← you are here
├── scripts/
│   ├── init-db.sql             # PostgreSQL extensions (pg_trgm, pgcrypto, pg_stat_statements)
│   ├── seed-db.sh              # Idempotent seed data
│   └── test-e2e.sh             # Local E2E test runner
├── references/                 # Reference patterns referenced by SKILL.md
│   ├── architecture-patterns.md
│   ├── axum-patterns.md
│   ├── leptos-patterns.md
│   ├── postgres-patterns.md
│   └── testing-patterns.md
├── live-search/                # Leptos SSR + WASM hydrate + sqlx + SSE
│   ├── Cargo.toml              # ssr / hydrate / otel / dev-tools features
│   ├── migrations/             # sqlx migrations (search_results, trigram)
│   ├── benches/
│   │   └── db_bench.rs         # Criterion benchmark for FTS queries
│   └── src/
│       ├── main.rs             # Thin launcher → bootstrap → shutdown
│       ├── lib.rs              # Public module surface + hydrate entry
│       ├── app.rs              # Leptos components + server functions
│       ├── bootstrap.rs        # Pool, migrations, listener, axum router
│       ├── db.rs               # PgPool + PgListener + watchdog
│       ├── cache.rs            # moka in-memory search cache
│       ├── events.rs           # SseEvent types
│       ├── sse.rs              # SSE handler (broadcast → text/event-stream)
│       ├── shutdown.rs         # Graceful shutdown drain
│       ├── otel.rs             # OpenTelemetry init (feature-gated)
│       └── styles.rs           # stylance-scoped CSS classes
├── gateway/                    # Pure axum + JWT + service registry
│   ├── Cargo.toml              # package name `gateway-example`
│   ├── migrations/             # refresh_tokens table (wiring is TODO)
│   ├── benches/
│   │   └── auth_bench.rs       # Criterion JWT sign/verify benchmark
│   └── src/
│       ├── main.rs             # Bin entry point + ALLOW_DEV_KEYS guard
│       ├── lib.rs              # Public module surface
│       ├── gateway.rs          # GatewayState + router composition
│       ├── module.rs           # ServiceModule trait + ServiceHealthError
│       ├── cors.rs             # Shared CORS layer (ALLOWED_ORIGINS env)
│       ├── pem.rs              # PKCS#8 / SPKI DER / PEM encode
│       ├── settings.rs         # env-based Settings + --dev-keys
│       ├── sse.rs              # GatewayEvent + SSE broadcast
│       ├── openapi.rs          # utoipa schema + Swagger UI at /docs
│       ├── otel.rs             # OpenTelemetry init (feature-gated)
│       ├── auth/
│       │   ├── mod.rs          # Re-exports
│       │   ├── error.rs        # AppError + IntoResponse
│       │   ├── jwt.rs          # EdDSA JWT create/validate
│       │   ├── handlers.rs     # login / refresh / logout / protected
│       │   └── middleware.rs   # Bearer-token validation
│       └── services/
│           ├── mod.rs
│           ├── search.rs       # SearchService + health_check probe
│           ├── proxy.rs        # ProxyService + check_history
│           └── monitor.rs      # MonitorService dashboard
├── i18n-demo/                  # Compile-time-checked i18n showcase
│   ├── Cargo.toml              # ssr / hydrate features
│   ├── build.rs                # leptos_i18n codegen
│   ├── locales/                # JSON locale files (en.json, de.json)
│   └── src/
│       ├── main.rs             # SSR server
│       ├── lib.rs              # Public module surface + hydrate entry
│       ├── app.rs              # Leptos components (t! macros)
│       ├── i18n.rs             # Generated t!/Locale module
│       └── styles.rs           # CSS constants + include_str!
└── e2e-tests/                  # chromiumoxide-based browser E2E
    ├── Cargo.toml              # browser-tests feature
    ├── tests/
    │   ├── live_search_test.rs # SSR + search + live feed + SSE
    │   ├── sse_test.rs         # SSE event delivery + insta snapshot
    │   └── gateway_test.rs     # Gateway endpoints + auth
    └── src/
        ├── lib.rs              # Shared helpers (base_url, join_url)
        └── common/
            ├── mod.rs          # Public re-exports
            ├── once.rs         # SharedServer<T> lazy one-shot bootstrap
            ├── chromium.rs     # setup/teardown + locate_chrome()
            ├── live_search_env.rs   # In-process live-search launcher
            ├── gateway_env.rs       # In-process gateway launcher
            ├── db.rs           # testcontainer Postgres fixture
            └── json.rs         # (any JSON helpers)
```

## Crate Details

### `live-search` (Leptos + Axum SSR + PostgreSQL)

- **Dependencies**: Leptos 0.8, leptos_axum, Axum 0.8, sqlx 0.9 (PostgreSQL), tokio, tokio-stream, serde, uuid, chrono
- **Features**:
  - `ssr` — Server-side rendering (required for the server binary)
  - `hydrate` — WASM hydration (for client-side binary)
- **Demonstrates**:
  - Full-text search with PostgreSQL `tsvector` / GIN indexes
  - `LISTEN`/`NOTIFY` via sqlx PgListener for real-time updates
  - SSE push from Axum to Leptos reactive signals
  - sqlx migrations (compile-time checked queries)
  - Fine-grained reactivity with Leptos 0.8 signals
- **Port**: 3000

### `gateway-example` (Axum + JWT + ServiceModule)

- **Dependencies**: Axum 0.8, Tower 0.5, jsonwebtoken 10, uuid
- **Demonstrates**:
  - `ServiceModule` trait for swappable service backends
  - JWT authentication middleware as Tower layer
  - Multi-service route composition with CORS
  - Health-check endpoint and modular error handling
- **Port**: 3001

### `e2e-tests` (chromiumoxide)

- **Dependencies**: chromiumoxide 0.9, reqwest 0.13, tokio, serde_json
- **Demonstrates**:
  - Browser automation with chromiumoxide
  - Real DOM interaction and assertion
  - SSE event stream validation
  - Cross-service integration testing
- **Note**: Set `CHROME_PATH=/path/to/chrome-or-chromium` if browser autodetection does not find a usable binary.

## Troubleshooting

### `relation "search_results" does not exist`
Run `./scripts/seed-db.sh` to initialize the schema and seed data.

### Workspace build fails
Run the workspace, SSR, and hydrate checks because the `live-search` server
binary and client bundle are intentionally gated behind separate features:
```bash
cargo check --workspace --all-targets
cargo check -p live-search --features ssr --all-targets
rustup target add wasm32-unknown-unknown
cargo check -p live-search --target wasm32-unknown-unknown --features hydrate --lib
```

### `aws-lc-rs` fails to compile
Install `cmake` and a C compiler (`build-essential` on Debian/Ubuntu, `base-devel` on Arch,
`Xcode Command Line Tools` on macOS). `aws-lc-rs` requires a C toolchain at build time
(the `aws-lc-sys` C library is bundled and compiled from source).

### E2E tests fail
- Ensure PostgreSQL is running and seeded.
- Ensure Chrome or Chromium is installed, or set `CHROME_PATH`.
- Run `./scripts/test-e2e.sh` for the full automated pipeline.

## License

MIT — see [LICENSE](LICENSE) for details.

---

*Part of the `rust-web-fullstack` skill showcase. Repo: https://git.onhir.eu/rust-web-fullstack*
