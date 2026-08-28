//! Graceful shutdown handling for the live-search server.
//!
//! Shutdown owns the same resources startup created: the cancellation token,
//! background-task `JoinSet`, `PostgreSQL` pool, and optional telemetry provider.

use std::time::Duration;

use tokio::signal;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::db;

/// Wait for Ctrl+C/SIGTERM, cancel the application, drain owned tasks, then
/// close the database pool and telemetry provider.
///
/// Order matters: background tasks are drained before the pool is closed so the
/// `PgListener` can observe cancellation and release its connection cleanly.
///
/// # Panics
/// Only panics if OS signal-handler installation fails.
///
/// # Errors
/// Returns the first background-task error/panic observed while draining, or an
/// error if tasks exceed the shutdown grace period. Cleanup still runs before
/// the error is returned.
#[expect(
    clippy::expect_used,
    reason = "signal handler installation failure is an unrecoverable runtime state"
)]
pub async fn wait(
    shutdown: CancellationToken,
    tasks: &mut JoinSet<anyhow::Result<()>>,
    pool: &sqlx::PgPool,
) -> anyhow::Result<()> {
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

    shutdown.cancelled().await;
    shutdown.cancel();

    let drain = tokio::time::timeout(Duration::from_secs(10), async {
        let mut first_error: Option<anyhow::Error> = None;
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(error = %error, "background task completed with an error");
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Err(join_error) => {
                    tracing::error!(
                        error = ?join_error,
                        is_panic = join_error.is_panic(),
                        "background task did not complete cleanly"
                    );
                    if first_error.is_none() {
                        first_error = Some(anyhow::Error::new(join_error));
                    }
                }
            }
        }
        first_error
    })
    .await;

    let shutdown_error = match drain {
        Ok(error) => error,
        Err(_elapsed) => {
            tracing::warn!("background tasks did not drain within 10s; aborting");
            tasks.abort_all();
            tokio::time::sleep(Duration::from_millis(250)).await;
            Some(anyhow::anyhow!(
                "background tasks exceeded the 10 second shutdown grace period"
            ))
        }
    };

    db::close_pool(pool).await;

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

    shutdown_error.map_or_else(|| Ok(()), Err)
}
