# rust-web-fullstack

A working showcase of **Leptos 0.8 + PostgreSQL + Axum** patterns from the `rust-web-fullstack` skill.
Three example crates demonstrating full-stack Rust patterns with server-side rendering,
hydration assets, real-time database notifications, service-oriented routing, and end-to-end testing.

## What It Demonstrates

| Crate | Pattern | Highlights |
|-------|---------|------------|
| **live-search** | Full-stack Leptos SSR + PostgreSQL FTS | PgListener + SSE push, sqlx migrations, full-text search with `tsvector`, reactive UI |
| **gateway** | ServiceModule trait + multi-service composition | JWT auth middleware, Tower service layers, mock modular services |
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

┌──────────────────┐
│   Gateway (:3001)│
│   JWT Auth       │
│   Mock services  │
│   ServiceModule  │
└──────────────────┘
```

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
├── Cargo.toml                  # Workspace manifest
├── Cargo.lock
├── docker-compose.yml          # PostgreSQL + both services
├── live-search.Dockerfile      # Multi-stage build for live-search
├── gateway.Dockerfile          # Multi-stage build for gateway
├── Makefile                    # Common development commands
├── .env.example                # Environment variable template
├── .woodpecker.yml             # CI workflow (Forgejo / Woodpecker)
├── .gitignore
├── LICENSE                     # MIT
├── README.md                   # ← you are here
├── scripts/
│   ├── init-db.sql             # PostgreSQL schema + triggers
│   ├── seed-db.sh              # Sample data seeder
│   └── test-e2e.sh             # E2E test runner
├── live-search/                # Leptos SSR + PostgreSQL FTS + SSE
│   ├── Cargo.toml
│   ├── migrations/             # sqlx migrations
│   └── src/
│       ├── main.rs             # Axum server entrypoint
│       ├── lib.rs              # Leptos app + routing
│       ├── db.rs               # sqlx queries + PgListener
│       ├── sse.rs              # SSE stream handler
│       └── app.rs              # Leptos components and routes
├── gateway/                    # ServiceModule trait + JWT auth
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs             # Axum server with modular routes
│       ├── auth.rs             # JWT middleware
│       ├── services/           # ServiceModule implementations
│       └── gateway.rs          # Dynamic route composition
└── e2e-tests/                  # chromiumoxide end-to-end tests
    ├── Cargo.toml
    └── tests/
        ├── live_search_test.rs # UI interaction tests
        ├── sse_test.rs         # SSE stream validation
        └── gateway_test.rs     # Gateway API tests
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

### E2E tests fail
- Ensure PostgreSQL is running and seeded.
- Ensure Chrome or Chromium is installed, or set `CHROME_PATH`.
- Run `./scripts/test-e2e.sh` for the full automated pipeline.

## License

MIT — see [LICENSE](LICENSE) for details.

---

*Part of the `rust-web-fullstack` skill showcase. Repo: https://git.onhir.eu/rust-web-fullstack*
