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
/// # Errors
/// Returns an error if an OS signal handler cannot be installed, if a background
/// task fails or panics while draining, or if tasks exceed the shutdown grace
/// period. Cleanup still runs before any of these errors is returned.
pub async fn wait(
    shutdown: CancellationToken,
    tasks: &mut JoinSet<anyhow::Result<()>>,
    pool: &sqlx::PgPool,
) -> anyhow::Result<()> {
    let mut signal_error: Option<anyhow::Error> = None;

    #[cfg(unix)]
    match signal::unix::signal(signal::unix::SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                () = shutdown.cancelled() => {
                    tracing::info!("shutdown requested by application");
                }
                result = signal::ctrl_c() => {
                    match result {
                        Ok(()) => tracing::info!("Ctrl+C received, initiating shutdown"),
                        Err(error) => {
                            tracing::error!(%error, "failed to install or await Ctrl+C handler");
                            signal_error = Some(error.into());
                        }
                    }
                }
                _ = terminate.recv() => {
                    tracing::info!("SIGTERM received, initiating shutdown");
                }
            }
        }
        Err(error) => {
            tracing::error!(%error, "failed to install SIGTERM handler");
            signal_error = Some(error.into());
        }
    }

    #[cfg(not(unix))]
    tokio::select! {
        () = shutdown.cancelled() => {
            tracing::info!("shutdown requested by application");
        }
        result = signal::ctrl_c() => {
            match result {
                Ok(()) => tracing::info!("Ctrl+C received, initiating shutdown"),
                Err(error) => {
                    tracing::error!(%error, "failed to install or await Ctrl+C handler");
                    signal_error = Some(error.into());
                }
            }
        }
    }

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

    let drain_error = match drain {
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

    signal_error.or(drain_error).map_or_else(|| Ok(()), Err)
}
