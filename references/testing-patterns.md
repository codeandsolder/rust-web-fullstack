# Testing Patterns Reference

Companion to [`SKILL.md`](../SKILL.md). Prefer the executable helpers in
`e2e-tests/src/common/` and the current CI pipeline over copied snippets.

## Test layers

Use the cheapest layer that proves the behavior:

| Layer | Best for |
|---|---|
| unit tests | pure parsing, hashing, invariants, DTO behavior |
| in-process HTTP + testcontainers | Axum routing, server functions, DB/auth flows |
| chromiumoxide browser E2E | hydration, DOM behavior, JS event wiring, SSE in a real browser |
| Docker build/smoke | image contents, runtime paths, deployment wiring |

Do not use a browser for an API-only assertion, but do not replace a browser test
with a fake text route when the behavior being claimed is frontend hydration.

## PostgreSQL isolation

`TestEnv::postgres()` starts a fresh Postgres container and runs the **root
`migrations/` history**, the same history used by the production services.

This is important: tests must not create a second migration universe or bypass
SQLx bookkeeping with raw DDL merely to make fixtures start.

Seed data required by a test explicitly. Browser tests must not depend on another
test having inserted rows first, especially when `--test-threads` or test order
changes.

## In-process live-search fixture

`LiveSearchEnv` mounts the actual Leptos route list/shell and server-function
catch-all. The fixture may inject an isolated database and test-specific static
asset directory, but `/` and `/live` must represent the real application router.

A fixture like:

```rust
Router::new().route("/", get(|| async { "test fixture" }))
```

cannot validate that the production page contains a search input or that
hydration works.

## Browser requirements

chromiumoxide controls an existing Chrome/Chromium executable through CDP.
`Browser::launch` does **not** require chromiumoxide's `bytes` feature.

This repository's helper reads `CHROME_PATH`:

```bash
CHROME_PATH=/usr/bin/chromium
```

CI installs Chromium explicitly. Do not document a machine-specific Playwright
cache build number as canonical compatibility guidance.

The browser event handler must be continuously pumped while tests run; otherwise
CDP work can stall.

## Hydration-aware input

For Leptos `bind:value`, setting an input in browser automation must cause the DOM
`input` event the application listens for. The canonical helper sets the value
through the native input setter and dispatches one bubbleable `input` event.

Do not assume a library's `type_str()` helper has exactly the browser event
semantics required by the framework without verifying it.

## DOM selectors

Selectors must match the current rendered markup. `live-search` result rows use
`data-testid="result-item"`; the title is rendered as a link inside the table,
not as an `<h3>`.

Prefer stable test IDs for structural assertions and assert user-visible text
only where the text is itself part of the contract.

## Search tests

Browser search tests seed their own known rows, navigate to the actual app, fill
the hydrated input, submit, and wait for `[data-testid="result-item"]`.

For no-result tests, use a unique nonsense query and wait for the explicit
"No results found." state.

Because the UI supports debounced automatic search and explicit submit, tests
should not assume that clicking submit is the only possible dispatch. The app
itself de-duplicates equivalent dispatches.

## SSE browser synchronization

Do not synchronize an SSE test on “the EventSource constructor returned.” The
client renders Connected only after receiving the server's named `connected`
event.

Canonical browser flow:

1. navigate to `/live`,
2. wait until `[data-testid="sse-status"]` contains `Connected`,
3. insert a unique sentinel row into the fixture database,
4. wait for that exact sentinel under `#live-results`,
5. clean it up best-effort.

This proves the complete chain:

```text
PostgreSQL insert
→ trigger/NOTIFY
→ PgListener
→ broadcast
→ Axum SSE
→ browser EventSource
→ Leptos reactive DOM
```

The browser client subscribes to all named events it handles (`connected`,
`search_result`, `stream_lagged`).

## Static asset tests

A successful SSR response is not proof that hydration assets exist. CI should
assert that built JS/WASM/CSS exists and that `/pkg/*` serves it.

The Stylance CSS test must inspect real output. `str::contains()` is literal, not
a regex engine; do not write checks such as:

```rust
line.contains("-[0-9a-fA-F]") // literal string, not a pattern
```

Use a real parser/regex or a deterministic property of the generated file.

Asset tests that CI claims to run must not be left `#[ignore]` unless the CI
command explicitly executes ignored tests.

## HTTP auth tests

The demo login request uses a UUID admin identity, not a username like `"admin"`:

```json
{
  "user_id": "00000000-0000-0000-0000-000000000001",
  "password": "..."
}
```

The configured admin UUID and password must match. Refresh uses a typed request:

```json
{ "refresh_token": "opaque-token" }
```

Useful auth assertions include:

- wrong/non-admin subject returns 401,
- wrong password returns 401,
- successful login persists a refresh-token digest,
- rotating a token revokes/replaces the prior row,
- replaying an old token revokes its family,
- logout revokes refresh state but does not claim to invalidate an already-issued
  access JWT before its expiry.

## No silent skips

If a test is part of required CI coverage, missing Docker, Chromium, build
artifacts, or a required server is a test failure. Silent helpers such as
`check_server_or_skip()` turn broken infrastructure into false green coverage.

Optional developer-only tests may be explicitly ignored with a clear reason, but
that should not be how the advertised CI suite is implemented.

## Timeouts and polling

Every external wait should have a timeout. Poll for a condition rather than
sleeping a fixed long duration when possible. A short hydration grace period can
be acceptable when there is no stable framework marker, but the actual assertion
must still poll for the desired condition.

## CI contract

The Woodpecker browser job must provide:

- Docker daemon access for testcontainers,
- Chromium at `CHROME_PATH`,
- production Leptos build artifacts at `LIVE_SEARCH_PKG_DIR`,
- single-threaded browser test execution where shared browser/server fixtures
  require it.

The Docker build job also needs a real Docker daemon/CLI pairing; invoking
`docker build` from a plain Rust image without a socket/service is not a Docker
build test.

## Screenshots

chromiumoxide screenshot APIs return byte buffers directly; this is independent
of whether `Browser::launch` needs a `bytes` Cargo feature (it does not). Store
screenshots only when visual output is the assertion; DOM/state assertions are
usually less brittle.
