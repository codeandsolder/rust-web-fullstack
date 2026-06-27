#!/usr/bin/env bash
# ===========================================================================
# run-e2e-tests.sh
#
# End-to-end test runner for the e2e-tests crate.
#
#  1. Start live-search (port 3000) and gateway (port 3001) if not already
#     running, with proper environment variables.
#  2. Wait for both services to become healthy.
#  3. Run the Rust integration tests.
#  4. Clean up background processes on exit.
#
# Usage:
#   ./scripts/run-e2e-tests.sh
#
# Environment variables (all optional):
#   DATABASE_URL      PostgreSQL connection string
#                     (default: postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo)
#   CARGO_FLAGS       Extra flags for cargo test (e.g. "--release")
# ===========================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
E2E_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$E2E_DIR/.." && pwd)"

# ── Configuration ────────────────────────────────────────────────────────────
DATABASE_URL="${DATABASE_URL:-postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo}"
CARGO_FLAGS="${CARGO_FLAGS:-}"

LIVE_SEARCH_PORT=3000
GATEWAY_PORT=3001

# Track PIDs for cleanup
LIVE_SEARCH_PID=""
GATEWAY_PID=""

# ── Cleanup handler ─────────────────────────────────────────────────────────
cleanup() {
    echo ""
    echo "==> Cleaning up background processes..."
    if [ -n "$LIVE_SEARCH_PID" ]; then
        echo "   Stopping live-search (PID $LIVE_SEARCH_PID)"
        kill "$LIVE_SEARCH_PID" 2>/dev/null || true
    fi
    if [ -n "$GATEWAY_PID" ]; then
        echo "   Stopping gateway (PID $GATEWAY_PID)"
        kill "$GATEWAY_PID" 2>/dev/null || true
    fi
    wait 2>/dev/null || true
    echo "==> Cleanup done."
}
trap cleanup EXIT INT TERM

# ── Helper: wait for an HTTP 200 ────────────────────────────────────────────
wait_for_healthy() {
    local url="$1"
    local label="$2"
    local max_attempts="${3:-30}"

    echo "   Waiting for $label at $url ..."
    for i in $(seq 1 "$max_attempts"); do
        if curl -sf "$url" > /dev/null 2>&1; then
            echo "   $label is ready."
            return 0
        fi
        sleep 1
    done
    echo "   ERROR: $label did not become healthy within ${max_attempts}s at $url"
    return 1
}

# ── Start live-search (if not already running) ──────────────────────────────
echo "==> Checking live-search on port $LIVE_SEARCH_PORT ..."
if curl -sf "http://localhost:${LIVE_SEARCH_PORT}/" > /dev/null 2>&1; then
    echo "   live-search is already running."
else
    echo "==> Starting live-search ..."
    cd "$REPO_DIR"
    DATABASE_URL="$DATABASE_URL" \
        cargo run $CARGO_FLAGS --release -p live-search --features ssr &
    LIVE_SEARCH_PID=$!
    cd "$E2E_DIR"

    wait_for_healthy "http://localhost:${LIVE_SEARCH_PORT}/" "live-search" 60
fi

# ── Start gateway (if not already running) ──────────────────────────────────
echo "==> Checking gateway on port $GATEWAY_PORT ..."
if curl -sf "http://localhost:${GATEWAY_PORT}/health" > /dev/null 2>&1; then
    echo "   gateway is already running."
else
    echo "==> Starting gateway ..."
    cd "$REPO_DIR"
    cargo run $CARGO_FLAGS --release -p gateway &
    GATEWAY_PID=$!
    cd "$E2E_DIR"

    wait_for_healthy "http://localhost:${GATEWAY_PORT}/health" "gateway" 60
fi

# ── Run integration tests ───────────────────────────────────────────────────
echo ""
echo "==> Running E2E integration tests ..."
echo ""

# Phase 1: live-search / SSE tests (port 3000)
echo "--- live-search + SSE tests (BASE_URL=http://localhost:${LIVE_SEARCH_PORT}) ---"
BASE_URL="http://localhost:${LIVE_SEARCH_PORT}" \
    cargo test $CARGO_FLAGS -p e2e-tests \
        --features integration \
        -- \
        --test-threads=1 \
        --nocapture \
        integration_live_search_ integration_sse_

echo ""
echo "--- gateway tests (BASE_URL=http://localhost:${GATEWAY_PORT}) ---"
BASE_URL="http://localhost:${GATEWAY_PORT}" \
    cargo test $CARGO_FLAGS -p e2e-tests \
        --features integration \
        -- \
        --test-threads=1 \
        --nocapture \
        integration_gateway_

echo ""
echo "==> All E2E integration tests completed."
