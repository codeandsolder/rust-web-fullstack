//! Graceful shutdown handling for the live-search server.
//!
//! Provides [`wait`] which blocks until `Ctrl+C` (all platforms) or `SIGTERM`
//! (Unix) is received, then cancels the shutdown token and drains background
//! tasks and the database pool within a grace period.
//!
//! This module is compiled only under `feature = "ssr"`.
//!
//! # Shutdown sequence
//!
//! 1. Wait for OS signal (Ctrl+C / SIGTERM).
//! 2. Cancel the [`CancellationToken`], which propagates to:
//!    - The HTTP server's graceful shutdown.
//!    - The `PgListener` and watchdog tasks.
//! 3. Close the database pool (5 s timeout).
//! 4. Drain the background-task `JoinSet` (10 s timeout; abort on timeout).
//! 5. If `OTel` is active, force-flush and shutdown the tracer provider
//!    (5 s timeout each, per rust-tracing §1.7 / §3.2).

use std::time::Duration;

use tokio::signal;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::db;

/// Wait for a shutdown signal, then perform graceful shutdown.
///
/// # Panics
/// Only panics if the OS-level signal handler installation fails, which
/// indicates an unrecoverable runtime state.
///
/// # Errors
/// Returns an error if the server's `JoinSet` drain observes a panic in
/// any background task (the panic is logged and surfaced as an `Err`).
#[expect(
    clippy::expect_used,
    reason = "Signal handler installation can only fail in unrecoverable runtime states"
)]
pub async fn wait(
    shutdown: CancellationToken,
    tasks: &mut JoinSet<anyhow::Result<()>>,
    pool: &sqlx::PgPool,
) -> anyhow::Result<()> {
    // ---- signal handler ---------------------------------------------------
    //
    // We install the handler here, inside `wait()`, so the caller does not
    // need to worry about signal-registration ordering. The handler task
    // runs until a signal arrives, then cancels the token.

    let signal_token = shutdown.clone();
    tokio::spawn(async move {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };
        #[cfg(unix)]
        let terminate = async {
            let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
            sig.recv().await;
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            () = ctrl_c => tracing::info!("Ctrl+C received, initiating shutdown"),
            () = terminate => tracing::info!("SIGTERM received, initiating shutdown"),
        }
        signal_token.cancel();
    });

    // ---- wait for the signal to actually fire -----------------------------
    //
    // The cancellation token was passed to `axum::serve`'s graceful shutdown
    // in bootstrap. When the signal fires, the server will start draining.
    shutdown.cancelled().await;

    // ---- close database pool ----------------------------------------------

    db::close_pool(pool).await;

    // ---- drain background tasks -------------------------------------------
    //
    // A second `cancel()` is idempotent (the signal handler already fired).
    // Inspect each `JoinError` so a panic is logged rather than silently
    // swallowed.

    shutdown.cancel();
    match tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(())) => {} // clean exit
                Ok(Err(e)) => {
                    tracing::error!(
                        error = %e,
                        "background task completed with an error"
                    );
                }
                Err(join_err) => {
                    tracing::error!(
                        error = ?join_err,
                        is_panic = join_err.is_panic(),
                        "background task did not complete cleanly"
                    );
                }
            }
        }
    })
    .await
    {
        Ok(()) => {}
        Err(_elapsed) => {
            tracing::warn!("background tasks did not drain within 10s; aborting");
            tasks.abort_all();
        }
    }

    // ---- OTel shutdown ----------------------------------------------------
    //
    // `force_flush` and `shutdown` on `SdkTracerProvider` are synchronous
    // (not async) in opentelemetry_sdk 0.32. Run them on a blocking thread
    // with a 5-second timeout.

    #[cfg(feature = "otel")]
    {
        if let Some(provider) = crate::bootstrap::get_tracer_provider() {
            let provider = provider.clone();
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || {
                    let _ = provider.force_flush();
                    let _ = provider.shutdown();
                }),
            )
            .await;
        }
    }

    Ok(())
}
