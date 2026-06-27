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

mod common;

use common::{base_url, require_server, setup, teardown};

/// Example integration test that waits for a server and performs a basic
/// browser navigation.
///
/// Replace `description_here` with a meaningful name.  The `integration_`
/// prefix makes it easy to filter with `cargo test integration_`.
#[tokio::test]
#[cfg_attr(
    not(feature = "integration"),
    ignore = "requires --features integration"
)]
async fn integration_example() {
    // ── 1. Server availability check ──────────────────────────────
    // If the server isn't running, fail immediately.
    require_server(&base_url()).await;

    // ── 2. Test logic ─────────────────────────────────────────────
    let ctx = setup().await;

    // Navigate, interact, assert …
    // e.g.
    //   ctx.page.goto_builder(&ctx.base_url).goto().await.unwrap();
    //   let title = ctx.page.title().await.unwrap();
    //   assert!(!title.is_empty());

    teardown(ctx).await;
}

// ── Additional tests follow the same pattern ─────────────────────
// #[tokio::test]
// #[cfg_attr(not(feature = "integration"), ignore = "requires --features integration")]
// async fn integration_another_test() {
//     require_server(&base_url()).await;
//     let ctx = setup().await;
//     // …
//     teardown(ctx).await;
// }
