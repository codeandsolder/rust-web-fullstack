#![allow(
    clippy::unused_async_trait_impl,
    reason = "Leptos #[server] generates client-side trait impl stubs without awaits when compiled without ssr"
)]

use leptos::prelude::*;
#[allow(
    clippy::wildcard_imports,
    reason = "leptos_meta re-exports are feature-gated (ssr/csr/hydrate); wildcard avoids conditional import errors when features change"
)]
use leptos_meta::*;
use leptos_router::components::{FlatRoutes, Route, Router};
use leptos_router::path;

use crate::db::SearchResult;
#[cfg(target_arch = "wasm32")]
use crate::events::SseEvent;

// ---------------------------------------------------------------------------
// Server function: search via PostgreSQL full-text search
// ---------------------------------------------------------------------------

#[server(endpoint = "/api/search")]
pub async fn search(query: String) -> Result<Vec<SearchResult>, ServerFnError> {
    use crate::db::get_pool;

    let Some(pool) = get_pool() else {
        return Err(ServerFnError::ServerError(
            "database pool is not initialized".to_string(),
        ));
    };

    sqlx::query_as::<_, SearchResult>(
        r"SELECT id, title, url, snippet, created_at
           FROM search_results
           WHERE fts @@ plainto_tsquery('english', $1)
           ORDER BY created_at DESC
           LIMIT 20",
    )
    .bind(&query)
    .fetch_all(pool)
    .await
    .map_err(|e| ServerFnError::ServerError(e.to_string()))
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
            <nav style="margin-bottom: 1rem; padding: 0.5rem; border-bottom: 1px solid #ccc;">
                <a href="/">"Search"</a>
                " | "
                <a href="/live">"Live Feed"</a>
            </nav>
            <main style="padding: 0.5rem;">
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
    let search_action = Action::new(|input: &String| {
        let input = input.clone();
        async move { search(input).await }
    });

    // Flatten the last result into something we can iterate over.
    let results = move || {
        search_action
            .value()
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
    };

    view! {
        <h2>"Search"</h2>
        <form
            on:submit=move |ev| {
                ev.prevent_default();
                search_action.dispatch(query.get());
            }
            style="margin-bottom: 1rem;"
        >
            <input
                type="text"
                placeholder="Enter search query..."
                bind:value=(query, set_query)
                style="width: 300px; padding: 0.4rem;"
            />
            <button type="submit" style="padding: 0.4rem 1rem; margin-left: 0.5rem;">
                "Search"
            </button>
        </form>

        <div id="results">
            {move || {
                let items = results();
                if items.is_empty() && search_action.value().get().is_some() {
                    // A search was submitted but returned no results.
                    view! { <p>"No results found."</p> }.into_any()
                } else if items.is_empty() {
                    view! { <p>"Enter a query above to search."</p> }.into_any()
                } else {
                    items
                        .into_iter()
                        .map(|r| {
                            let url = r.url.clone();
                            view! {
                                <div class="result-item" style="margin-bottom: 0.8rem; padding: 0.5rem; border: 1px solid #eee; border-radius: 4px;">
                                    <h3 style="margin: 0 0 0.2rem;">
                                        <a href={url}>{r.title.clone()}</a>
                                    </h3>
                                    <p style="margin: 0 0 0.2rem;">{r.snippet.clone()}</p>
                                    <small style="color: #666;">{r.url}</small>
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

// ---------------------------------------------------------------------------
// Live-feed page – receives search results via SSE as they are inserted
// ---------------------------------------------------------------------------

/// A display-friendly result (subset of fields from `SearchResult`).
#[derive(Debug, Clone)]
struct LiveResult {
    title: String,
    url: String,
    snippet: String,
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
    //
    // # Panics
    // Panics if the `EventSourceSubscription` stream returns a value that
    // cannot be destructured as `(String, String)`. This is infallible in
    // practice because the underlying implementation always yields
    // `Ok((String, String))` for well-formed SSE messages.
    #[cfg(target_arch = "wasm32")]
    {
        let es = gloo_net::eventsource::futures::EventSource::new("/api/events");
        match es {
            Ok(mut event_source) => {
                leptos::task::spawn_local(async move {
                    use futures::stream::StreamExt;

                    let Ok(mut stream) = event_source.subscribe("message") else {
                        leptos::logging::error!("Failed to subscribe to SSE message events");
                        return;
                    };

                    while let Some(Ok((_event_type, msg))) = stream.next().await {
                        let Some(data) = msg.data().as_string() else {
                            leptos::logging::warn!("SSE message had non-string data");
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
                                    let result = LiveResult {
                                        title,
                                        url,
                                        snippet,
                                    };
                                    results.update(|r| r.push(result));
                                }
                                SseEvent::StreamLagged { skipped } => {
                                    leptos::logging::warn!(
                                        "SSE stream lagged by {skipped} messages"
                                    );
                                }
                            },
                            Err(e) => {
                                leptos::logging::warn!("Invalid SSE message: {e:?}");
                            }
                        }
                    }
                });
            }
            Err(e) => {
                leptos::logging::warn!("Failed to connect to SSE: {e:?}");
            }
        }
    }

    view! {
        <h2>"Live Feed"</h2>
        <p>
            "Results appear below in real time as they are inserted into the database."
        </p>
        {move || {
            if connected.get() {
                view! { <p style="color: green;">"✓ Connected to live feed"</p> }.into_any()
            } else {
                view! { <p style="color: #888;">"Connecting …"</p> }.into_any()
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
                                <div class="result-item" style="margin-bottom: 0.8rem; padding: 0.5rem; border: 1px solid #eee; border-radius: 4px;">
                                    <h3 style="margin: 0 0 0.2rem;">{r.title.clone()}</h3>
                                    <p style="margin: 0 0 0.2rem;">{r.snippet.clone()}</p>
                                    <small style="color: #666;">{r.url.clone()}</small>
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
