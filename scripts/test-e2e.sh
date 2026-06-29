#!/usr/bin/env bash
# Start live-search and gateway, wait for them to be ready, run E2E tests.
# Builds all binaries once, then runs them.
set -euo pipefail
cd "$(dirname "$0")/.."
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
docker compose exec -T postgres pg_isready -U rwf -d rwf_demo
echo "==> Building all binaries (one build, no cargo run)..."
cargo build --release --locked -p live-search --features ssr
cargo build --release --locked -p gateway-example
echo "==> Applying database migrations..."
cargo install sqlx-cli --version 0.8.4 --no-default-features --features postgres,rustls --locked
DATABASE_URL="postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo" sqlx migrate run --source live-search/migrations
echo "==> Seeding database..."
./scripts/seed-db.sh "postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo"
echo "==> Starting live-search..."
DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  ./target/release/live-search &
LIVE_SEARCH_PID=$!
trap "kill $LIVE_SEARCH_PID 2>/dev/null || true" EXIT
echo "==> Waiting for live-search on :3000..."
for i in {1..30}; do
  if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -sf http://localhost:3000/ > /dev/null
echo "==> Starting gateway..."
./target/release/gateway-example &
GATEWAY_PID=$!
trap "kill $LIVE_SEARCH_PID $GATEWAY_PID 2>/dev/null || true" EXIT
echo "==> Waiting for gateway on :3001..."
for i in {1..30}; do
  if curl -sf http://localhost:3001/health > /dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -sf http://localhost:3001/health > /dev/null
echo "==> Running E2E tests..."
CHROME_PATH=$CHROME_PATH BASE_URL=http://localhost:3000 cargo test --locked -p e2e-tests --features integration --test live_search_test -- --test-threads=1 --nocapture
CHROME_PATH=$CHROME_PATH BASE_URL=http://localhost:3000 cargo test --locked -p e2e-tests --features integration --test sse_test -- --test-threads=1 --nocapture
CHROME_PATH=$CHROME_PATH BASE_URL=http://localhost:3001 cargo test --locked -p e2e-tests --features integration --test gateway_test -- --test-threads=1 --nocapture
echo "==> Tests complete."
