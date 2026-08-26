#!/usr/bin/env bash
# Start live-search and gateway, wait for them to be ready, run E2E tests.
# Builds all binaries once (including the WASM bundle live-search needs for
# hydration), then runs them.
set -euo pipefail
cd "$(dirname "$0")/.."

# ── Env vars required by gateway::settings::Settings::load() ─────────────
# Must export ADMIN_PASSWORD BEFORE starting the gateway, otherwise it
# refuses to start.
: "${ADMIN_PASSWORD:=synthetic-gateway-test-password}"
export ADMIN_PASSWORD

echo "==> Checking CHROME_PATH..."
if [ -z "${CHROME_PATH:-}" ]; then
  if command -v chromium &>/dev/null; then
    export CHROME_PATH=$(command -v chromium)
  elif command -v chromium-browser &>/dev/null; then
    export CHROME_PATH=$(command -v chromium-browser)
  elif command -v google-chrome &>/dev/null; then
    export CHROME_PATH=$(command -v google-chrome)
  else
    echo "ERROR: CHROME_PATH is not set and no Chrome binary found in PATH."
    echo "Set CHROME_PATH to the chromium/chrome executable or install Chromium."
    exit 1
  fi
fi
echo "  CHROME_PATH=$CHROME_PATH"
echo "==> Starting PostgreSQL..."
docker compose up -d postgres
echo "==> Waiting for PostgreSQL to be healthy..."
for i in {1..30}; do
  if docker compose exec -T postgres pg_isready -U rwf -d rwf_demo; then
    break
  fi
  sleep 1
done
if ! docker compose exec -T postgres pg_isready -U rwf -d rwf_demo >/dev/null 2>&1; then
  echo "ERROR: postgres not ready after 30s"
  exit 1
fi
echo "==> Building all binaries (one build, no cargo run)..."
cargo build --release --locked -p live-search --features ssr
cargo build --release --locked -p gateway-example
echo "==> Building WASM hydration bundle (live-search needs ./pkg for SSR hydration)..."
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
cargo install stylance-cli --locked
cargo build --release --locked -p live-search --lib \
  --target wasm32-unknown-unknown --features hydrate

# Use cargo-leptos's site layout so the test fixture (LIVE_SEARCH_PKG_DIR)
# finds JS, WASM, and CSS in one place.
LIVE_SEARCH_SITE_PKG=./live-search/target/site/pkg
mkdir -p "$LIVE_SEARCH_SITE_PKG"
wasm-bindgen --target web --out-dir "$LIVE_SEARCH_SITE_PKG" --out-name live_search \
  target/wasm32-unknown-unknown/release/live_search.wasm
# Stylance build produces the real hashed-class CSS in site/pkg/.
stylance build 2>/dev/null || stylance build --manifest-path live-search/Cargo.toml
echo "==> Applying database migrations..."
DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  ./target/release/live-search &
MIGRATION_PID=$!

LIVE_SEARCH_PID=""
GATEWAY_PID=""
PKG_DIR="$LIVE_SEARCH_SITE_PKG"
cleanup() {
    local rc=$?
    [ -n "${GATEWAY_PID:-}" ] && kill "$GATEWAY_PID" 2>/dev/null || true
    [ -n "${LIVE_SEARCH_PID:-}" ] && kill "$LIVE_SEARCH_PID" 2>/dev/null || true
    [ -n "${MIGRATION_PID:-}" ] && kill "$MIGRATION_PID" 2>/dev/null || true
    # Targeted cleanup via tracked PIDs only — NO broad `pkill -f` (which
    # would kill any other developer's local server matching the string).
    return $rc
}
trap cleanup EXIT INT TERM

echo "==> Waiting for live-search (migration runner) on :3000..."
for i in {1..30}; do
  if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
    break
  fi
  sleep 1
done
if ! curl -sf http://localhost:3000/ > /dev/null; then
  echo "ERROR: live-search migration runner did not start within 30s"
  exit 1
fi
kill $MIGRATION_PID 2>/dev/null || true
wait $MIGRATION_PID 2>/dev/null || true
MIGRATION_PID=""
echo "==> Seeding database..."
./scripts/seed-db.sh "postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo"
echo "==> Starting live-search..."
DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  LEPTOS_OUTPUT_NAME=live_search \
  LIVE_SEARCH_PKG_DIR="$PKG_DIR" \
  ./target/release/live-search &
LIVE_SEARCH_PID=$!
echo "==> Waiting for live-search on :3000..."
for i in {1..30}; do
  if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
    break
  fi
  sleep 1
done
if ! curl -sf http://localhost:3000/ > /dev/null; then
  echo "ERROR: live-search did not become healthy within 30s"
  exit 1
fi
echo "==> Starting gateway..."
ALLOW_DEV_KEYS=1 ./target/release/gateway-example --dev-keys &
GATEWAY_PID=$!
echo "==> Waiting for gateway on :3001..."
for i in {1..30}; do
  if curl -sf http://localhost:3001/health > /dev/null 2>&1; then
    break
  fi
  sleep 1
done
if ! curl -sf http://localhost:3001/health > /dev/null; then
  echo "ERROR: gateway did not become healthy within 30s"
  exit 1
fi
echo "==> Running E2E tests..."
CHROME_PATH=$CHROME_PATH BASE_URL=http://localhost:3000 \
  DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  LIVE_SEARCH_PKG_DIR="$PKG_DIR" \
  cargo test --release --locked -p e2e-tests --features browser-tests \
    --test live_search_test -- --test-threads=1 --nocapture
CHROME_PATH=$CHROME_PATH BASE_URL=http://localhost:3001 \
  DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  cargo test --release --locked -p e2e-tests --features browser-tests \
    --test gateway_test -- --test-threads=1 --nocapture
echo "==> Tests complete."