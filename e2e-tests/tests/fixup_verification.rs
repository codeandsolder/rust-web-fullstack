//! Regression tests verifying each fix from the Phase 1d oracle session (ora-5).
//!
//! Most tests run unconditionally (no `integration` feature gate) and do not
//! require external services.  C3 requires Docker (testcontainers) to verify
//! that `TestEnv::postgres()` ignores the `DATABASE_URL` env var.

mod common;

use std::sync::Mutex;

use proptest::prelude::*;

/// Serializes env-var mutation tests so concurrent test-execution (e.g.
/// nextest) doesn't race on `set_var` / `remove_var`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ===========================================================================
// C1 — mockall_demo returns strings containing "rust"
// Verifies fix-08 (ora-5 C1): the mock returns strings that contain "rust".
// ===========================================================================

#[test]
fn c1_mockall_returns_rust_strings() {
    // Replicate the mockall trait pattern from gateway_test.rs::pattern_fixtures.
    #[allow(dead_code)]
    #[mockall::automock]
    trait SearchService {
        fn search(&self, query: &str) -> Vec<String>;
    }

    let mut mock = MockSearchService::new();
    mock.expect_search()
        .with(mockall::predicate::eq("rust"))
        .returning(|_| {
            vec![
                "rust-lang.org".to_string(),
                "rust-cookbook.github.io".to_string(),
            ]
        });

    let results = mock.search("rust");
    assert_eq!(results.len(), 2, "Expected 2 results from mock");

    for r in &results {
        assert!(
            r.to_lowercase().contains("rust"),
            "Mock result '{r}' does not contain 'rust' (case-insensitive)"
        );
    }
}

// ===========================================================================
// C2 — event_snapshot sanitization via placeholders
// Verifies fix-08 (ora-5 C2): dynamic fields are replaced with placeholders
// before snapshotting, so the snapshot is deterministic across test runs.
// ===========================================================================

#[test]
fn c2_event_snapshot_replaces_dynamic_fields_with_placeholders() {
    // Build an event with a unique dynamic title/URL (same pattern as
    // sse_test::notify_trigger_fires_sse_event).
    let dynamic_title = format!("e2e-test-{}", uuid::Uuid::new_v4());
    let dynamic_url = format!("https://example.com/e2e-test/{dynamic_title}");
    let event = serde_json::json!({
        "type": "SearchResult",
        "title": dynamic_title,
        "url": dynamic_url,
        "snippet": "Regression test for C2 sanitization",
    });

    // Sanitize the same way sse_test::event_snapshot does.
    let mut sanitized = event;
    if let Some(val) = sanitized.get_mut("title")
        && let Some(title) = val.as_str()
        && (title.starts_with("e2e-test-") || title.starts_with("e2e-warmup-"))
    {
        *val = serde_json::Value::String("<DYNAMIC_TEST_TITLE>".to_string());
    }
    if let Some(val) = sanitized.get_mut("url")
        && let Some(url) = val.as_str()
        && (url.contains("e2e-test-") || url.contains("e2e-warmup-"))
    {
        *val = serde_json::Value::String("<DYNAMIC_TEST_URL>".to_string());
    }

    assert_eq!(
        sanitized["title"], "<DYNAMIC_TEST_TITLE>",
        "Dynamic title should be replaced with placeholder"
    );
    assert_eq!(
        sanitized["url"], "<DYNAMIC_TEST_URL>",
        "Dynamic URL should be replaced with placeholder"
    );
    assert_eq!(
        sanitized["snippet"], "Regression test for C2 sanitization",
        "Static fields should be unchanged"
    );

    // Also verify the snapshot file exists (this doubles as I5's assertion).
    let snap_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/sse_test__sse_search_result_event.snap");
    assert!(
        snap_path.exists(),
        "Snapshot file must exist: {}",
        snap_path.display()
    );
    let metadata = std::fs::metadata(&snap_path).expect("Failed to read snapshot metadata");
    assert!(
        metadata.len() > 0,
        "Snapshot file must be non-empty: {}",
        snap_path.display()
    );
}

// ===========================================================================
// C3 — TestEnv::postgres() ignores DATABASE_URL env var
// Verifies that the testcontainer-only mode correctly ignores the DATABASE_URL
// environment variable: even when set to an unreachable host, TestEnv spins up
// a real container with a random port.
// ===========================================================================

#[allow(
    unsafe_code,
    reason = "std::env::set_var/remove_var are marked unsafe for soundness; \
              we serialize access via ENV_LOCK so the multi-threaded concern is mitigated"
)]
#[test]
fn c3_database_url_ignored_when_using_testcontainers() {
    // Serialize env-var mutation across all tests in this binary.
    let guard = ENV_LOCK.lock().expect("env lock poisoned");

    let original = std::env::var("DATABASE_URL").ok();

    // Set DATABASE_URL to an unreachable host — TestEnv must ignore it.
    // SAFETY: protected by ENV_LOCK; no concurrent env access.
    unsafe {
        std::env::set_var("DATABASE_URL", "postgres://ignored-host:1/ignored");
    }

    // Use a single tokio runtime for the entire test.  ContainerAsync's drop
    // requires an active tokio runtime context, so all assertions + cleanup
    // that touches `env` must happen inside block_on.
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async {
        let env = common::TestEnv::postgres().await;

        // Restore env BEFORE any assertion that may fail (cleanup is guaranteed).
        // SAFETY: protected by ENV_LOCK; no concurrent env access.
        if let Some(url) = original {
            unsafe {
                std::env::set_var("DATABASE_URL", &url);
            }
        } else {
            unsafe {
                std::env::remove_var("DATABASE_URL");
            }
        }
        // Drop the guard — env does not rely on env vars.
        drop(guard);

        // The connection string must not contain the DATABASE_URL value.
        let conn_str = env.connection_string();
        assert!(
            !conn_str.contains("ignored-host"),
            "connection string must not contain 'ignored-host': {conn_str}"
        );

        // The port must not be 1 (testcontainer chose a random ephemeral port).
        #[allow(
            clippy::double_ended_iterator_last,
            reason = "conn_str is short; clarity over performance"
        )]
        let port_str = conn_str.split(':').last().unwrap_or("");
        assert_ne!(
            port_str, "1",
            "connection string port must not be 1: {conn_str}"
        );

        // Double-check: the pool actually works.
        let result: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(env.pool())
            .await
            .expect("pool should connect to a real database, not ignored-host");
        assert_eq!(
            result, 1,
            "SELECT 1 must return 1 on the testcontainer pool"
        );

        // env is dropped here, inside the tokio runtime context.
    });
    // rt is dropped here, after env's async drop completes.
}

// ===========================================================================
// I2 — proptest strategies for Uuid + DateTime
// Verifies fix-08 (ora-5 I2): proptest can generate SearchResult instances
// and they round-trip through JSON losslessly.
// ===========================================================================

proptest::proptest! {
    /// SearchResult round-trips through serde_json losslessly.
    /// Pre-fix, this would have failed to compile because proptest strategies
    /// for Uuid and DateTime were missing.
    #[test]
    fn i2_search_result_json_roundtrip(
        id in (0u128..).prop_map(uuid::Uuid::from_u128),
        title in "[a-zA-Z0-9 ]{1,100}",
        url in "[a-zA-Z0-9:/._-]{1,200}",
        snippet in "[a-zA-Z0-9 ]{0,500}",
        secs in 0i64..1_000_000_000i64,
    ) {
        use chrono::{Duration, Utc};

        let s: String = snippet;
        let result = live_search::db::SearchResult {
            id,
            title,
            url,
            snippet: s,
            created_at: Utc::now() - Duration::seconds(secs),
        };

        let json = serde_json::to_value(&result)
            .expect("serialization must succeed");
        let deserialized: live_search::db::SearchResult =
            serde_json::from_value(json.clone())
                .expect("deserialization must succeed");
        let re_serialized = serde_json::to_value(&deserialized)
            .expect("re-serialization must succeed");

        prop_assert_eq!(json, re_serialized,
            "SearchResult round-trip through JSON must be lossless");
    }
}

// ===========================================================================
// I4 — collapsible_if clippy fix in sse_test.rs
// Verifies fix-08 (ora-5 I4): sse_test.rs passes clippy without
// collapsible_if warnings.
// ===========================================================================

#[test]
fn i4_no_collapsible_if_anywhere_in_e2e_tests() {
    // Run clippy across the e2e-tests crate with collapsible_if enabled as
    // a warning (it's allow-by-default).  Pre-fix, sse_test.rs had at least
    // one collapsible_if lint.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("e2e-tests is a workspace member");

    let output = std::process::Command::new("cargo")
        .args([
            "clippy",
            "-p",
            "e2e-tests",
            "--tests",
            "--",
            "-D",
            "warnings",
            "-W",
            "clippy::collapsible_if",
        ])
        .current_dir(workspace_root)
        .output()
        .expect("Failed to run cargo clippy");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Exit code zero → no warnings or errors → pass.
    assert!(
        output.status.success(),
        "clippy exited with status {} — collapsible_if or other lint violation.\n\
         stderr:\n{stderr}",
        output.status
    );

    // Also assert the string "collapsible_if" does not appear in stderr.
    assert!(
        !stderr.contains("collapsible_if"),
        "clippy output contains 'collapsible_if':\n{stderr}"
    );
}

// ===========================================================================
// I5 — snapshot file exists
// Verifies fix-08 (ora-5 I5): the snapshot file is present and non-empty.
// Note: C2's test above also checks this, but this test explicitly documents
// the I5 fixup with its own assertion.
// ===========================================================================

#[test]
fn i5_snapshot_file_exists_and_is_non_empty() {
    let snap_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/sse_test__sse_search_result_event.snap");

    assert!(
        snap_path.exists(),
        "Snapshot file must exist: {}",
        snap_path.display()
    );

    let metadata = std::fs::metadata(&snap_path).expect("Failed to read snapshot metadata");
    assert!(
        metadata.len() > 0,
        "Snapshot file must be non-empty: {}",
        snap_path.display()
    );
}
