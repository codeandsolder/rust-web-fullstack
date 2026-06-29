//! Template / pattern example for future integration tests.
//!
//! Copy this file as a starting point when adding a new integration test file.
//! Each integration test:
//!
//! 1. Requires that the target server is reachable via [`require_server`].
//!    If the server is not running the test fails immediately.
//!
//! 2. Is gated behind the `integration` Cargo feature so it is ignored by
//!    plain `cargo test`.  Run with `cargo test --features integration` to
//!    execute integration tests.
//!
//! 3. Uses `--test-threads=1` to avoid contention between browser-based tests.
//!
//! Usage in the test runner script (`scripts/run-e2e-tests.sh`):
//!
//! ```bash
//! BASE_URL=http://localhost:3000 \
//!   cargo test -p e2e-tests --features integration -- --test-threads=1 --nocapture
//! ```
//!
//! Helper functions that do NOT require a browser (`base_url`, `join_url`) live
//! in the `e2e_tests` lib crate.  Browser-bound helpers (`setup`, `teardown`,
//! `require_server`) live in `common::*`.

mod common;

use common::{require_server, setup, teardown};
use e2e_tests::base_url;

/// Example integration test that waits for a server and performs a basic
/// browser navigation.
///
/// Replace the function name with a meaningful name.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_example() {
    // ── 1. Server availability check ──────────────────────────────
    // If the server isn't running, fail immediately.
    require_server(&base_url(None)).await;

    // ── 2. Test logic ─────────────────────────────────────────────
    let ctx = setup().await;

    // Verify the server is still reachable and navigate to the homepage.
    require_server(&ctx.base_url).await;
    ctx.page
        .goto(&ctx.base_url)
        .await
        .expect("Failed to navigate to base_url");

    // Assert the homepage actually loaded (title is non-empty).
    let title = ctx
        .page
        .get_title()
        .await
        .expect("Failed to read page title")
        .unwrap_or_default();
    assert!(!title.is_empty(), "Page title should not be empty");

    teardown(ctx).await;
}

// ── Additional tests follow the same pattern ─────────────────────
// #[tokio::test]
// #[cfg_attr(not(feature = "integration"), ignore = "requires --features integration")]
// async fn integration_another_test() {
//     require_server(&base_url(None)).await;
//     let ctx = setup().await;
//     // …
//     teardown(ctx).await;
// }
