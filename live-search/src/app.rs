//! Leptos UI components and server functions for the live-search frontend.
//!
//! Two pages are provided via `FlatRoutes`:
//! - `/` — [`SearchPage`] with full-text + trigram search.
//! - `/live` — [`LiveFeedPage`] with named Server-Sent Events.

use std::collections::VecDeque;
use std::sync::Arc;

use leptos::prelude::*;
use leptos::{hydration::AutoReload, hydration::HydrationScripts};
use leptos_meta::{MetaTags, Stylesheet, Title, provide_meta_context};
use leptos_router::components::{FlatRoutes, Route, Router};
use leptos_router::path;

use lepticons::{Icon, LucideGlyph};
use leptos_struct_table::{EventHandler, TableContent, TableDataProvider, TableRow};
use leptos_use::watch_debounced;

use crate::db::SearchResult;
#[cfg(target_arch = "wasm32")]
use crate::events::SseEvent;
use crate::styles;

/// Search `search_results` using PostgreSQL FTS with a trigram fallback.
///
/// The `title % $1` branch is backed by the `pg_trgm` GIN index created by
/// migration 003, so the advertised typo-tolerant behavior is real rather than
/// an unused index. Internal database errors are logged server-side and are not
/// exposed to clients.
///
/// # Errors
/// Returns a server-function error for invalid input, unavailable application
/// state, or a database failure.
#[server(endpoint = "search")]
pub async fn search(query: String) -> Result<Arc<Vec<SearchResult>>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::db::SearchResultRow;
        use crate::state;

        let Some(ctx) = state::get() else {
            return Err(ServerFnError::ServerError(
                "search service unavailable".to_string(),
            ));
        };

        let trimmed = query.trim().to_lowercase();
        let len = trimmed.chars().count();
        if !(1..=1024).contains(&len) {
            return Err(ServerFnError::ServerError(
                "query must be 1..=1024 characters".into(),
            ));
        }

        if let Some(cached) = ctx.cache.get(&trimmed).await {
            return Ok(cached);
        }

        let query_result = sqlx::query_as::<_, SearchResultRow>(
            r"WITH q AS (SELECT plainto_tsquery('english', $1) AS tsq)
              SELECT id, title, url, snippet, created_at
              FROM search_results, q
              WHERE fts @@ q.tsq OR title % $1
              ORDER BY
                (fts @@ q.tsq) DESC,
                GREATEST(ts_rank_cd(fts, q.tsq), similarity(title, $1)) DESC,
                created_at DESC,
                id DESC
              LIMIT 20",
        )
        .bind(&trimmed)
        .fetch_all(&ctx.pool)
        .await;

        let results: Vec<SearchResult> = match query_result {
            Ok(rows) => rows.into_iter().map(Into::into).collect(),
            Err(e) => {
                tracing::error!(error = %e, "search query failed");
                return Err(ServerFnError::ServerError(
                    "search service unavailable".into(),
                ));
            }
        };

        let results = Arc::new(results);
        ctx.cache.insert(trimmed, results.clone()).await;
        Ok(results)
    }
    #[cfg(not(feature = "ssr"))]
    {
        let _ = query;
        Err(ServerFnError::ServerError(
            "search() server fn called on a non-ssr build".into(),
        ))
    }
}

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

#[must_use]
#[expect(
    clippy::must_use_candidate,
    reason = "Leptos #[component] generates a wrapper that nightly Clippy evaluates separately"
)]
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet href="/pkg/live-search.css" />
        <Title text="Live Search" />

        <Router>
            <nav class=styles::nav>
                <a class=styles::nav_link href="/">
                    <Icon glyph=LucideGlyph::Search size="16" />
                    " Search"
                </a>
                " | "
                <a class=styles::nav_link href="/live">
                    <Icon glyph=LucideGlyph::Radio size="16" />
                    " Live Feed"
                </a>
            </nav>
            <main class=styles::main>
                <FlatRoutes fallback=|| view! { <p>"Page not found."</p> }>
                    <Route path=path!("/") view=SearchPage />
                    <Route path=path!("/live") view=LiveFeedPage />
                </FlatRoutes>
            </main>
        </Router>
    }
}

#[component]
#[expect(
    clippy::empty_enums,
    reason = "Leptos #[component] generates an empty typed-builder state enum"
)]
fn SearchErrorBoundary(children: Children) -> impl IntoView {
    view! {
        <ErrorBoundary fallback=move |_errors| {
            view! {
                <div class="error-boundary" data-testid="error-boundary">
                    <h3>"Something went wrong."</h3>
                    <p>"Try reloading the page to recover."</p>
                </div>
            }
        }>{children()}</ErrorBoundary>
    }
}

#[derive(Debug, TableRow, Clone)]
#[table(impl_vec_data_provider)]
pub struct SearchResultRow {
    #[table(title = "Title", renderer = "TitleLinkCellRenderer")]
    pub title: String,
    #[table(title = "Snippet")]
    pub snippet: String,
    #[table(title = "URL")]
    pub url: String,
}

#[component]
#[expect(
    clippy::empty_enums,
    reason = "Leptos #[component] generates an empty typed-builder state enum"
)]
fn TitleLinkCellRenderer(
    class: String,
    value: Signal<String>,
    row: RwSignal<SearchResultRow>,
    index: usize,
) -> impl IntoView {
    let _ = index;
    let url = move || row.read().url.clone();
    view! {
        <td class=class>
            <a href=url>{value}</a>
        </td>
    }
}

fn result_row_renderer(
    class: Signal<String>,
    row: RwSignal<SearchResultRow>,
    index: usize,
    _selected: Signal<bool>,
    on_select: EventHandler<leptos::web_sys::MouseEvent>,
    columns: RwSignal<Vec<usize>>,
) -> impl IntoView {
    view! {
        <tr class=class data-testid="result-item" on:click=move |ev| on_select.run(ev)>
            {SearchResultRow::render_row(row, index, columns)}
        </tr>
    }
}

#[must_use]
#[expect(
    clippy::must_use_candidate,
    reason = "Leptos #[component] generates a wrapper that nightly Clippy evaluates separately"
)]
#[component]
pub fn SearchPage() -> impl IntoView {
    let (query, set_query) = signal(String::new());

    let search_action = Action::new(|input: &String| {
        let input = input.clone();
        async move { search(input).await }
    });
    let last_dispatched = RwSignal::new(String::new());

    // Both debounce and explicit submit go through the same de-duplication
    // guard, avoiding the old double request when the user clicked Search at
    // roughly the same time as the 300 ms debounce fired.
    let dispatch_search = move |value: String| {
        let normalized = value.trim().to_string();
        if normalized.is_empty() || normalized == last_dispatched.get_untracked() {
            return;
        }
        last_dispatched.set(normalized.clone());
        search_action.dispatch(normalized);
    };
    let dispatch_debounced = dispatch_search;
    let _stop = watch_debounced(
        move || query.get(),
        move |new_query, _old_query, _| dispatch_debounced(new_query.clone()),
        300.0,
    );

    let action_value = move || search_action.value().get();
    let has_query = move || !query.get().trim().is_empty();

    view! {
        <h2>"Search"</h2>

        <form
            on:submit=move |ev| {
                ev.prevent_default();
                dispatch_search(query.get());
            }
            class=styles::form
        >
            <input
                type="text"
                placeholder="Enter search query..."
                bind:value=(query, set_query)
                data-testid="search-input"
                class=styles::input
            />
            <button type="submit" data-testid="search-submit" class=styles::button>
                "Search"
            </button>
        </form>

        <SearchErrorBoundary>
            <div id="results">
                <Show when=move || !has_query() fallback=|| ()>
                    <p>"Enter a query above to search."</p>
                </Show>
                <Show when=move || has_query() && search_action.pending().get() fallback=|| ()>
                    <p data-testid="search-pending">"Searching …"</p>
                </Show>

                {move || {
                    if !has_query() {
                        return None;
                    }
                    action_value()
                        .and_then(Result::err)
                        .map(|e| view! { <p class="error">{e.to_string()}</p> })
                }}

                {move || {
                    if !has_query() {
                        return None;
                    }
                    action_value()
                        .and_then(Result::ok)
                        .map(|items| {
                            if items.is_empty() {
                                view! { <p class=styles::empty>"No results found."</p> }.into_any()
                            } else {
                                let rows: Vec<SearchResultRow> = items
                                    .iter()
                                    .map(|r| SearchResultRow {
                                        title: r.title.clone(),
                                        snippet: r.snippet.clone(),
                                        url: r.url.clone(),
                                    })
                                    .collect();
                                view! {
                                    <table class=styles::table>
                                        <TableContent
                                            rows
                                            scroll_container=""
                                            row_renderer=result_row_renderer
                                        />
                                    </table>
                                }
                                    .into_any()
                            }
                        })
                }}
            </div>
        </SearchErrorBoundary>
    }
}

#[derive(Debug, Clone)]
struct LiveResult {
    title: Arc<str>,
    url: Arc<str>,
    snippet: Arc<str>,
}

#[cfg(target_arch = "wasm32")]
fn apply_sse_event(
    event: SseEvent,
    results: RwSignal<VecDeque<LiveResult>>,
    connected: RwSignal<bool>,
) {
    use leptos::logging;

    match event {
        SseEvent::Connected { .. } => connected.set(true),
        SseEvent::SearchResult {
            title,
            url,
            snippet,
        } => {
            results.update(|items| {
                if items.len() >= 200 {
                    items.pop_front();
                }
                items.push_back(LiveResult {
                    title,
                    url,
                    snippet,
                });
            });
        }
        SseEvent::StreamLagged { skipped } => {
            logging::warn!("SSE stream lagged by {skipped} messages");
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn consume_event_source(
    event_source: &mut gloo_net::eventsource::futures::EventSource,
    results: RwSignal<VecDeque<LiveResult>>,
    connected: RwSignal<bool>,
    stop: RwSignal<bool>,
) {
    use futures::{StreamExt, stream};
    use gloo_net::eventsource::State;
    use gloo_timers::future::sleep;
    use leptos::logging;
    use std::time::Duration;

    let connected_stream = event_source.subscribe("connected");
    let result_stream = event_source.subscribe("search_result");
    let lagged_stream = event_source.subscribe("stream_lagged");
    let (Ok(connected_events), Ok(result_events), Ok(lagged_events)) =
        (connected_stream, result_stream, lagged_stream)
    else {
        logging::error!("Failed to subscribe to named SSE events");
        return;
    };

    while event_source.state() == State::Connecting && !stop.get() {
        sleep(Duration::from_millis(25)).await;
    }
    if stop.get() {
        return;
    }
    if event_source.state() != State::Open {
        logging::warn!("SSE connection closed before opening");
        return;
    }

    let data_events = stream::select(result_events, lagged_events);
    let mut all_events = stream::select(connected_events, data_events);
    while let Some(result) = all_events.next().await {
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
                    Ok(event) => apply_sse_event(event, results, connected),
                    Err(error) => logging::warn!("Invalid SSE message: {error:?}"),
                }
            }
            Err(error) => {
                logging::warn!("SSE stream error: {error:?}");
                return;
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn run_live_feed(
    results: RwSignal<VecDeque<LiveResult>>,
    connected: RwSignal<bool>,
    stop: RwSignal<bool>,
) {
    use gloo_net::eventsource::futures::EventSource;
    use gloo_timers::future::sleep;
    use leptos::logging;
    use std::time::Duration;

    loop {
        if stop.get() {
            logging::log!("SSE live feed stopped");
            return;
        }

        match EventSource::new("/api/events") {
            Ok(mut event_source) => {
                consume_event_source(&mut event_source, results, connected, stop).await;
                connected.set(false);
                event_source.close();
            }
            Err(error) => logging::warn!("Failed to create EventSource: {error}"),
        }

        if stop.get() {
            return;
        }
        sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(target_arch = "wasm32")]
fn start_live_feed(results: RwSignal<VecDeque<LiveResult>>, connected: RwSignal<bool>) {
    let stop = RwSignal::new(false);
    let stop_cleanup = stop;
    on_cleanup(move || stop_cleanup.set(true));
    leptos::task::spawn_local(run_live_feed(results, connected, stop));
}

#[must_use]
#[expect(
    clippy::must_use_candidate,
    reason = "Leptos #[component] generates a wrapper that nightly Clippy evaluates separately"
)]
#[component]
pub fn LiveFeedPage() -> impl IntoView {
    let results = RwSignal::new(VecDeque::<LiveResult>::new());
    let connected = RwSignal::new(false);

    #[cfg(target_arch = "wasm32")]
    start_live_feed(results, connected);

    view! {
        <h2>"Live Feed"</h2>
        <p>"Results appear below in real time as they are inserted into the database."</p>
        {move || {
            if connected.get() {
                view! {
                    <p class=styles::connected data-testid="sse-status">
                        "✓ Connected to live feed"
                    </p>
                }
                    .into_any()
            } else {
                view! {
                    <p class=styles::disconnected data-testid="sse-status">
                        "Connecting …"
                    </p>
                }
                    .into_any()
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
                        .map(|result| {
                            view! {
                                <div class=styles::result_item data-testid="live-result">
                                    <h3 class=styles::result_title>{result.title.clone()}</h3>
                                    <p class=styles::result_snippet>{result.snippet.clone()}</p>
                                    <small class=styles::result_url>{result.url.clone()}</small>
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
