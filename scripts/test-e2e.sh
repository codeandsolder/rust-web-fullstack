#!/usr/bin/env bash
# Start live-search and gateway, wait for them to be ready, run E2E tests.
set -euo pipefail

cd "$(dirname "$0")/.."

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

echo "==> Seeding database..."
./scripts/seed-db.sh postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo

echo "==> Starting live-search..."
DATABASE_URL=postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo \
  cargo run --release -p live-search --features ssr &
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
cargo run --release -p gateway-example &
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
BASE_URL=http://localhost:3000 cargo test -p e2e-tests --features integration --test live_search_test -- --test-threads=1 --nocapture
BASE_URL=http://localhost:3000 cargo test -p e2e-tests --features integration --test sse_test -- --test-threads=1 --nocapture
BASE_URL=http://localhost:3001 cargo test -p e2e-tests --features integration --test gateway_test -- --test-threads=1 --nocapture

echo "==> Tests complete."
