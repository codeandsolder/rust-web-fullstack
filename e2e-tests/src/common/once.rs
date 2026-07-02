//! Lazy one-shot server bootstrap shared across tests in a single binary.
//!
//! Provides [`SharedServer<T>`], a wrapper that runs an async initialisation
//! function exactly once on a dedicated background `tokio` runtime thread.
//! This ensures long-lived services (TCP listeners, database listeners, etc.)
//! survive the drop of individual test runtimes.
//!
//! # Example
//!
//! ```rust,ignore
//! use e2e_tests::common::once::SharedServer;
//! use e2e_tests::common::LiveSearchEnv;
//!
//! static SERVER: SharedServer<LiveSearchEnv> = SharedServer::new();
//!
//! async fn get_server() -> anyhow::Result<&'static LiveSearchEnv> {
//!     SERVER.get(|| async { LiveSearchEnv::start().await }).await
//! }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::OnceCell;

/// Lazy one-shot server bootstrap.
///
/// Runs the initialisation future exactly once on a dedicated background thread
/// with its own `tokio` runtime.  Subsequent calls to [`get`](Self::get) return
/// a reference to the already-initialised value.
///
/// The background runtime is kept alive indefinitely (via
/// `std::future::pending::<()>()`) so any background tasks spawned by the
/// initialiser survive individual test runtimes.
#[derive(Debug)]
pub struct SharedServer<T: Send + Sync + 'static> {
    /// Stores `Ok(env)` on success or `Err(arc_error)` on failure.
    cell: OnceCell<Result<T, Arc<anyhow::Error>>>,
    /// Signalled by the background thread once `cell` has been populated.
    bg_init_done: AtomicBool,
    /// Ensures the background thread is spawned exactly once.
    bg_init_once: Once,
}

impl<T: Send + Sync + 'static> SharedServer<T> {
    /// Create a new uninitialised server holder.
    ///
    /// This is `const`-callable so it can be used in a `static` initialiser.
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "Default is not meaningful for a once-only initialiser; callers should explicitly opt in via `static SERVER: SharedServer<T> = SharedServer::new()`"
    )]
    pub const fn new() -> Self {
        Self {
            cell: OnceCell::const_new(),
            bg_init_done: AtomicBool::new(false),
            bg_init_once: Once::new(),
        }
    }

    /// Get the shared server, starting it on a background thread if this is
    /// the first call.
    ///
    /// The `start` future is run on a dedicated `tokio` runtime so any
    /// long-lived tasks it spawns survive the caller's test runtime being
    /// dropped.
    ///
    /// # Errors
    /// Propagates any error from `start` (or from spawning the background
    /// thread or creating its runtime).
    pub async fn get<F, Fut>(&'static self, start: F) -> Result<&'static T>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    {
        self.bg_init_once.call_once(|| {
            // IIFE so we can use `?` inside a `call_once` closure (which
            // returns `()`).
            let result: Result<(), Arc<anyhow::Error>> = (|| {
                let handle = std::thread::Builder::new()
                    .name("e2e-bg-init".into())
                    .spawn(move || {
                        let rt = match tokio::runtime::Runtime::new() {
                            Ok(rt) => rt,
                            Err(e) => {
                                let err = Arc::new(
                                    anyhow::Error::new(e)
                                        .context("failed to create background init runtime"),
                                );
                                let _ = self.cell.set(Err(err));
                                self.bg_init_done.store(true, Ordering::Release);
                                return;
                            }
                        };
                        rt.block_on(async {
                            match start().await {
                                Ok(env) => {
                                    let _ = self.cell.set(Ok(env));
                                }
                                Err(e) => {
                                    let _ = self.cell.set(Err(Arc::new(e)));
                                }
                            }
                            self.bg_init_done.store(true, Ordering::Release);
                            if self.cell.get().is_none_or(Result::is_ok) {
                                // Keep the runtime alive indefinitely so any
                                // background management tasks spawned by the
                                // initialiser survive individual test runtimes.
                                std::future::pending::<()>().await;
                            }
                        });
                    })
                    .map_err(|e| {
                        Arc::new(
                            anyhow::Error::new(e).context("failed to spawn background init thread"),
                        )
                    })?;
                let _ = handle;
                Ok(())
            })();

            // If we could not even spawn the thread, surface the failure via
            // `self.cell` so the test-side loop sees the error instead of
            // timing out.
            if let Err(err) = result {
                let _ = self.cell.set(Err(err));
                self.bg_init_done.store(true, Ordering::Release);
            }
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !self.bg_init_done.load(Ordering::Acquire) {
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "background initialization timed out after 30s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let cell_ref = self.cell.get().context("server not initialized")?;
        cell_ref.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))
    }
}
