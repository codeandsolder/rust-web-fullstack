#!/usr/bin/env bash
# Start live-search and gateway, wait for them to be ready, run E2E tests.
# Builds all binaries once (including the WASM bundle live-search needs for
# hydration), then runs them.
set -euo pipefail
cd "$(dirname "$0")/.."

# ── Env vars required by gateway::settings::Settings::load() ─────────────
# Must export these BEFORE starting the gateway, otherwise it refuses to
# start with "ADMIN_PASSWORD must be set" / "JWT_SECRET must not be the
# placeholder value".
: "${ADMIN_PASSWORD:=synthetic-gateway-test-password}"
: "${JWT_SECRET:=dev-jwt-secret-change-in-production-please-32-bytes}"
export ADMIN_PASSWORD JWT_SECRET

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
cargo build --release --locked -p live-search --lib \
  --target wasm32-unknown-unknown --features hydrate
mkdir -p pkg
wasm-bindgen --target web --out-dir pkg --out-name live_search \
  target/wasm32-unknown-unknown/release/live_search.wasm
# The SSR shell renders <Stylesheet href="/pkg/live-search.css"/>; create an
# empty placeholder so the request returns 200 instead of 404.
touch pkg/live-search.css
echo "==> Applying database migrations..."
# Note: sqlx-cli 0.8.4 has been yanked from crates.io as of 2026; we instead
# run the live-search binary briefly to apply migrations via its embedded
# `sqlx::migrate!()` macro, then kill it and start the actual server below.
DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  ./target/release/live-search &
MIGRATION_PID=$!

# Single EXIT trap — installed AFTER every server PID is known, so the
# handler can reference every variable safely. (Earlier versions redefined
# the trap three times in different places, which works only by accident:
# bash evaluates trap-string variables at signal-fire time, not at
# registration time, so the second trap's `$GATEWAY_PID` was undefined when
# registered and only became defined by the time the trap actually fired.)
LIVE_SEARCH_PID=""
GATEWAY_PID=""
cleanup() {
    local rc=$?
    [ -n "$GATEWAY_PID" ] && kill "$GATEWAY_PID" 2>/dev/null || true
    [ -n "$LIVE_SEARCH_PID" ] && kill "$LIVE_SEARCH_PID" 2>/dev/null || true
    [ -n "$MIGRATION_PID" ] && kill "$MIGRATION_PID" 2>/dev/null || true
    # Best-effort: also kill anything still listening on 3000 / 3001 if our
    # PIDs were lost (e.g. after a `set -e` exit from a wait command).
    pkill -f "target/release/live-search" 2>/dev/null || true
    pkill -f "target/release/gateway-example" 2>/dev/null || true
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
MIGRATION_PID=""  # prevent trap from killing a PIDs we've already waited on
echo "==> Seeding database..."
./scripts/seed-db.sh "postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo"
echo "==> Starting live-search..."
DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  LEPTOS_OUTPUT_NAME=live_search \
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
./target/release/gateway-example &
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
  cargo test --release --locked -p e2e-tests --features integration \
    --test live_search_test -- --test-threads=1 --nocapture
CHROME_PATH=$CHROME_PATH BASE_URL=http://localhost:3000 \
  DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  cargo test --release --locked -p e2e-tests --features integration \
    --test sse_test -- --test-threads=1 --nocapture
CHROME_PATH=$CHROME_PATH BASE_URL=http://localhost:3001 \
  DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  cargo test --release --locked -p e2e-tests --features integration \
    --test gateway_test -- --test-threads=1 --nocapture
echo "==> Tests complete."
# Clean up the temporary WASM bundle directory so it doesn't pollute `git status`.
rm -rf pkg