//! Leptos UI components and server functions for the live-search frontend.
//!
//! Two pages are provided via `FlatRoutes`:
//! - `/` — [`SearchPage`] with a full-text search form backed by a server
//!   function.
//! - `/live` — [`LiveFeedPage`] that displays search results in real time
//!   via Server-Sent Events.

// Leptos #[server] generates client-side trait impl stubs without awaits
// when compiled without ssr — the unused async is expected.

use std::sync::Arc;

use leptos::prelude::*;
#[allow(
    clippy::wildcard_imports,
    reason = "leptos_meta re-exports are feature-gated (ssr/csr/hydrate); wildcard avoids conditional import errors when features change. `allow` (not `expect`) because whether the lint fires depends on which specific re-exports are used in each compiled target — `expect` would flag itself as unfulfilled when the lint doesn't fire."
)]
use leptos_meta::*;
use leptos_router::components::{FlatRoutes, Route, Router};
use leptos_router::path;

use lepticons::{Icon, LucideGlyph};
use leptos_use::watch_debounced;

use crate::db::SearchResult;
#[cfg(target_arch = "wasm32")]
use crate::events::SseEvent;
use crate::styles;

// ---------------------------------------------------------------------------
// Server function: search via PostgreSQL full-text search
// ---------------------------------------------------------------------------

/// Search `search_results` using PostgreSQL full-text search.
///
/// # Errors
///
/// Returns [`ServerFnError::ServerError`] if the query is empty / too long,
/// the global pool has not been initialized, or the database query fails.
#[allow(
    clippy::unused_async,
    reason = "The `#[server]` body has `.await` under `feature = \"ssr\"`. The non-ssr branch is synchronous (immediate error return) but the function must remain `async` for the server-fn macro's signature; this is the Leptos 0.8 idiom for SSR-gated server functions."
)]
#[server(endpoint = "/api/search")]
pub async fn search(query: String) -> Result<Arc<Vec<SearchResult>>, ServerFnError> {
    // The body touches `crate::db::get_pool` (gated by `feature = "ssr"`)
    // and uses `SearchResult`'s `sqlx::FromRow` derive, which only exists
    // under `feature = "ssr"`. Gate the DB-using body explicitly so that
    // `cargo check --workspace --all-targets` (which compiles with no
    // features active) does not see an unresolved `get_pool` import.
    //
    // The `#[server]` macro generates the client-side stub from the
    // function signature alone; this branch is only compiled in on the SSR
    // build. The non-ssr branch should never run — if it does, the
    // server-fn machinery has been bypassed and we surface the error to
    // the caller rather than panicking (workspace lint forbids `panic`).
    #[cfg(feature = "ssr")]
    {
        use crate::cache;
        use crate::db::get_pool;

        let trimmed = query.trim().to_lowercase();
        let len = trimmed.len();
        if !(1..=1024).contains(&len) {
            return Err(ServerFnError::ServerError(
                "query must be 1..=1024 characters".into(),
            ));
        }

        // Try the in-memory cache first.
        if let Some(cached) = cache::get(&trimmed).await {
            return Ok(cached);
        }

        let Some(pool) = get_pool() else {
            return Err(ServerFnError::ServerError(
                "database pool is not initialized".to_string(),
            ));
        };

        let query_result = sqlx::query_as::<_, SearchResult>(
            r"SELECT id, title, url, snippet, created_at
               FROM search_results
               WHERE fts @@ plainto_tsquery('english', $1)
               ORDER BY created_at DESC
               LIMIT 20",
        )
        .bind(&trimmed)
        .fetch_all(pool)
        .await;

        let results: Vec<SearchResult> = match query_result {
            Ok(r) => r,
            Err(e) => {
                return Err(ServerFnError::ServerError(e.to_string()));
            }
        };

        // Store in cache for subsequent requests.
        let results = Arc::new(results);
        cache::insert(trimmed, results.clone()).await;
        Ok(results)
    }
    #[cfg(not(feature = "ssr"))]
    {
        // Defensive: server fns are routed through the wire, never called
        // locally on the client. Returning a `ServerFnError` keeps the
        // workspace `clippy::panic = "deny"` rule satisfied.
        let _ = query;
        Err(ServerFnError::ServerError(
            "search() server fn called on a non-ssr build".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// App shell – used by the server for SSR
// ---------------------------------------------------------------------------

#[must_use]
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

// ---------------------------------------------------------------------------
// Root component
// ---------------------------------------------------------------------------

#[expect(
    clippy::must_use_candidate,
    reason = "Leptos #[component] converts this to a view fn; must_use is implicit"
)]
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet href="/pkg/live-search.css" />
        <Title text="Live Search" />

        <Router>
            <nav class={styles::nav}>
                <a class={styles::nav_link} href="/">
                    <Icon glyph=LucideGlyph::Search size="16" />" Search"
                </a>
                " | "
                <a class={styles::nav_link} href="/live">
                    <Icon glyph=LucideGlyph::Radio size="16" />" Live Feed"
                </a>
            </nav>
            <main class={styles::main}>
                <FlatRoutes fallback=|| view! { <p>"Page not found."</p> }>
                    <Route path=path!("/") view=SearchPage />
                    <Route path=path!("/live") view=LiveFeedPage />
                </FlatRoutes>
            </main>
        </Router>
    }
}

// ---------------------------------------------------------------------------
// Search page – submit a query, display results from the server function
// ---------------------------------------------------------------------------

#[expect(
    clippy::must_use_candidate,
    reason = "Leptos #[component] converts this to a view fn; must_use is implicit"
)]
#[component]
pub fn SearchPage() -> impl IntoView {
    let (query, set_query) = signal(String::new());

    // Watch the query input with a 300ms debounce so we don't fire a search
    // on every keystroke. The search is dispatched automatically when the
    // user stops typing.
    let search_action = Action::new(|input: &String| {
        let input = input.clone();
        async move { search(input).await }
    });
    let _stop = watch_debounced(
        move || query.get(),
        move |new_query, _old_query, _| {
            if !new_query.is_empty() {
                search_action.dispatch(new_query.clone());
            }
        },
        300.0,
    );

    // Track the last result's Ok and Err branches separately so the view
    // can render a distinct error branch instead of silently swallowing it.
    // Read `.value()` once per render frame; reading it twice would create
    // two reactive subscriptions and run the body twice on every change.
    let action_value = move || search_action.value().get();

    view! {
        <h2>"Search"</h2>
        <form
            on:submit=move |ev| {
                ev.prevent_default();
                search_action.dispatch(query.get());
            }
            class={styles::form}
        >
            <input
                type="text"
                placeholder="Enter search query..."
                bind:value=(query, set_query)
                class={styles::input}
            />
            <button type="submit" class={styles::button}>
                "Search"
            </button>
        </form>

        <div id="results">
            <Show when=move || action_value().is_none()
                fallback=|| ()>
                <p>"Enter a query above to search."</p>
            </Show>

            {move || action_value()
                .and_then(Result::err)
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
            }

            {move || action_value()
                .and_then(Result::ok)
                .map(|items| {
                    if items.is_empty() {
                        view! { <p>"No results found."</p> }.into_any()
                    } else {
                        (*items).clone()
                            .into_iter()
                            .map(|r| {
                                let url = r.url.clone();
                                view! {
                                    <div class={styles::result_item}>
                                        <h3 class={styles::result_title}>
                                            <a href={url}>{r.title}</a>
                                        </h3>
                                        <p class={styles::result_snippet}>{r.snippet}</p>
                                        <small class={styles::result_url}>{r.url}</small>
                                    </div>
                                }
                            })
                            .collect::<Vec<_>>()
                            .into_any()
                    }
                })
            }
        </div>
    }
}

// ---------------------------------------------------------------------------
// Live-feed page – receives search results via SSE as they are inserted
// ---------------------------------------------------------------------------

/// Display-friendly result held in the live-feed reactive list.
///
/// Uses `Arc<str>` to extend the `mem-arc-str` optimisation already in
/// `SseEvent::SearchResult` all the way to the rendering layer: the `Arc`
/// from the broadcast payload is moved into the list and displayed by
/// borrowing it. No per-event `String` allocation on the WASM side.
#[derive(Debug, Clone)]
struct LiveResult {
    title: Arc<str>,
    url: Arc<str>,
    snippet: Arc<str>,
}

#[expect(
    clippy::must_use_candidate,
    reason = "Leptos #[component] converts this to a view fn; must_use is implicit"
)]
#[component]
pub fn LiveFeedPage() -> impl IntoView {
    let results = RwSignal::new(Vec::<LiveResult>::new());
    let connected = RwSignal::new(false);

    // On the client (WASM) side, open an EventSource to the SSE endpoint.
    // The stream is long-lived; on disconnect or error we reconnect after a
    // 2-second delay.
    #[cfg(target_arch = "wasm32")]
    {
        let stop = RwSignal::new(false);
        let stop_cleanup = stop;
        on_cleanup(move || stop_cleanup.set(true));

        leptos::task::spawn_local(async move {
            use futures::stream::StreamExt;
            use gloo_timers::future::sleep;
            use leptos::logging;
            use std::time::Duration;

            loop {
                if stop.get() {
                    logging::log!("SSE live feed stopped");
                    return;
                }

                match gloo_net::eventsource::futures::EventSource::new("/api/events") {
                    Ok(mut event_source) => {
                        connected.set(true);

                        match event_source.subscribe("search_result") {
                            Ok(mut stream) => {
                                while let Some(result) = stream.next().await {
                                    if stop.get() {
                                        return;
                                    }
                                    match result {
                                        Ok((_event_type, msg)) => {
                                            let Some(data) = msg.data().as_string() else {
                                                logging::warn!("SSE message had non-string data");
                                                continue;
                                            };
                                            match serde_json::from_str::<SseEvent>(&data) {
                                                Ok(event) => match event {
                                                    SseEvent::Connected => {
                                                        connected.set(true);
                                                    }
                                                    SseEvent::SearchResult {
                                                        title,
                                                        url,
                                                        snippet,
                                                    } => {
                                                        results.update(|r| {
                                                            if r.len() >= 200 {
                                                                r.remove(0);
                                                            }
                                                            r.push(LiveResult {
                                                                title,
                                                                url,
                                                                snippet,
                                                            });
                                                        });
                                                    }
                                                    SseEvent::StreamLagged { skipped } => {
                                                        logging::warn!(
                                                            "SSE stream lagged by {skipped} messages"
                                                        );
                                                    }
                                                },
                                                Err(e) => {
                                                    logging::warn!("Invalid SSE message: {e:?}");
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            logging::warn!("SSE stream error: {e:?}");
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                logging::error!("Failed to subscribe to SSE search_result: {e:?}");
                            }
                        }

                        connected.set(false);
                    }
                    Err(e) => {
                        logging::warn!("Failed to connect to SSE: {e}");
                    }
                }

                // Reconnect delay — allow cancellation during sleep
                sleep(Duration::from_secs(2)).await;
            }
        });
    }

    view! {
        <h2>"Live Feed"</h2>
        <p>
            "Results appear below in real time as they are inserted into the database."
        </p>
        {move || {
            if connected.get() {
                view! { <p class={styles::connected}>"✓ Connected to live feed"</p> }.into_any()
            } else {
                view! { <p class={styles::disconnected}>"Connecting …"</p> }.into_any()
            }
        }}
        <div id="live-results">
            {move || {
                let items = results.get();
                if items.is_empty() {
                    view! { <p>"Waiting for results …"</p> }.into_any()
                } else {
                    items
                        .iter()
                        .map(|r| {
                            view! {
                                <div class={styles::result_item}>
                                    <h3 class={styles::result_title}>{r.title.clone()}</h3>
                                    <p class={styles::result_snippet}>{r.snippet.clone()}</p>
                                    <small class={styles::result_url}>{r.url.clone()}</small>
                                </div>
                            }
                        })
                        .collect::<Vec<_>>()
                        .into_any()
                }
            }}
        </div>
    }
}
