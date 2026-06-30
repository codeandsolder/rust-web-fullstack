//! Tests for the test infrastructure itself (testcontainers, fixtures, env).
//!
//! These tests don't exercise production code — they verify that the test
//! harness provides the isolation properties the rest of the suite depends on.

// silence: the common module is intentional; tests instantiate it
#![allow(unused_imports)]

mod common;

use std::sync::Mutex;

/// Serializes env-var mutation tests so concurrent test-execution (e.g.
/// nextest) doesn't race on `set_var` / `remove_var`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Verifies that [`common::TestEnv::postgres`] ignores any inherited
/// `DATABASE_URL` environment variable and always spins up a fresh
/// testcontainer on a random port.
#[allow(
    unsafe_code,
    reason = "std::env::set_var/remove_var are marked unsafe for soundness; \
              we serialize access via ENV_LOCK so the multi-threaded concern is mitigated"
)]
#[test]
fn database_url_env_var_is_ignored_by_testcontainers() -> anyhow::Result<()> {
    use anyhow::Context;

    // Serialize env-var mutation across all tests in this binary.
    let guard = ENV_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("env lock poisoned"))?;

    let original = std::env::var("DATABASE_URL").ok();

    // SAFETY: protected by ENV_LOCK; no concurrent env access.
    unsafe {
        std::env::set_var("DATABASE_URL", "postgres://ignored-host:1/ignored");
    }

    // ContainerAsync::drop requires an active tokio runtime context, so the
    // entire env lifecycle stays inside this block_on.
    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    let result: anyhow::Result<()> = rt.block_on(async {
        let env = common::TestEnv::postgres().await?;

        // Restore env BEFORE any assertion that might fail.
        if let Some(url) = original {
            // SAFETY: protected by ENV_LOCK.
            unsafe {
                std::env::set_var("DATABASE_URL", &url);
            }
        } else {
            // SAFETY: protected by ENV_LOCK.
            unsafe {
                std::env::remove_var("DATABASE_URL");
            }
        }
        drop(guard);

        // The connection string must NOT contain the DATABASE_URL value.
        let conn_str = env.connection_string();
        anyhow::ensure!(
            !conn_str.contains("ignored-host"),
            "connection string must not contain 'ignored-host': {conn_str}"
        );

        // The port must not be 1 (testcontainer chose a random ephemeral port).
        #[allow(
            clippy::double_ended_iterator_last,
            reason = "conn_str is short; clarity over performance"
        )]
        let port_str = conn_str.split(':').next_back().unwrap_or("");
        anyhow::ensure!(
            port_str != "1",
            "connection string port must not be 1: {conn_str}"
        );

        // Double-check: the pool actually works against the testcontainer.
        let one: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(env.pool())
            .await
            .context("pool should connect to a real database")?;
        anyhow::ensure!(one == 1, "SELECT 1 must return 1");

        Ok(())
        // env drops here, inside the tokio runtime.
    });
    result?;
    // rt drops here, after env's async drop completes.
    Ok(())
}
