#!/usr/bin/env bash
# Build the browser assets, verify Docker/Chromium prerequisites, then run the
# in-process testcontainers + chromiumoxide E2E suite. The tests start their own
# live-search/gateway fixtures; this script deliberately does not launch a second
# competing set of application servers.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Checking Docker..."
if ! docker info >/dev/null 2>&1; then
  echo "ERROR: a reachable Docker daemon is required for testcontainers."
  exit 1
fi

echo "==> Checking CHROME_PATH..."
if [ -z "${CHROME_PATH:-}" ]; then
  if command -v chromium >/dev/null 2>&1; then
    CHROME_PATH=$(command -v chromium)
  elif command -v chromium-browser >/dev/null 2>&1; then
    CHROME_PATH=$(command -v chromium-browser)
  elif command -v google-chrome >/dev/null 2>&1; then
    CHROME_PATH=$(command -v google-chrome)
  else
    echo "ERROR: set CHROME_PATH or install Chromium/Chrome."
    exit 1
  fi
fi
export CHROME_PATH
echo "  CHROME_PATH=$CHROME_PATH"

rustup target add wasm32-unknown-unknown

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "ERROR: wasm-bindgen CLI 0.2.126 is required."
  echo "Install it with: cargo install wasm-bindgen-cli --version 0.2.126 --locked"
  exit 1
fi
if ! command -v stylance >/dev/null 2>&1; then
  echo "ERROR: stylance-cli is required."
  echo "Install it with: cargo install stylance-cli --locked"
  exit 1
fi

echo "==> Building live-search SSR + hydration assets..."
cargo build --release --locked -p live-search --features ssr
cargo build --release --locked -p live-search --lib \
  --target wasm32-unknown-unknown --features hydrate

LIVE_SEARCH_PKG_DIR="${LIVE_SEARCH_PKG_DIR:-live-search/target/site/pkg}"
export LIVE_SEARCH_PKG_DIR
mkdir -p "$LIVE_SEARCH_PKG_DIR"
wasm-bindgen \
  --target web \
  --out-dir "$LIVE_SEARCH_PKG_DIR" \
  --out-name live_search \
  target/wasm32-unknown-unknown/release/live_search.wasm

# Keep the same Stylance invocation used by the Docker build. It writes the
# hashed stylesheet consumed by the Leptos shell/E2E asset assertions.
stylance build

test -s "$LIVE_SEARCH_PKG_DIR/live_search.js"
test -s "$LIVE_SEARCH_PKG_DIR/live_search_bg.wasm"
test -s "$LIVE_SEARCH_PKG_DIR/live-search.css"

echo "==> Running in-process E2E suite..."
cargo test --release --locked -p e2e-tests --tests --features browser-tests \
  -- --test-threads=1 --nocapture

echo "==> E2E tests complete."
