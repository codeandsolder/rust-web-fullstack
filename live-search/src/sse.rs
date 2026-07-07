//! Server-Sent Events handler and broadcast channel setup.
//!
//! A single [`broadcast::Sender<SseEvent>`] is created at startup and shared
//! between the `PgListener` task (producer) and all SSE client connections
//! (consumers). The [`sse_handler`] function streams events to HTTP clients.
//!
//! # Migration from globals
//!
//! Previously this module held a `static BROADCAST: OnceLock<…>` set via
//! [`set_broadcast`]. That global is now **removed** — the sender is passed
//! directly into [`sse_handler`] (axum route handlers wrap it in a closure).
//! Test and e2e setups do the same.

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::Utc;
use futures::Stream;
use futures::StreamExt;
use futures::future;
use futures::stream::{self, BoxStream};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::events::SseEvent;

/// SSE handler: streams events from the broadcast channel to the client.
///
/// The caller must provide the broadcast sender (typically captured by a
/// closure in the router setup).
#[allow(
    clippy::unused_async,
    reason = "Axum 0.8 requires async fn for Handler trait"
)]
pub async fn sse_handler(
    tx: broadcast::Sender<SseEvent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Emit a "Connected" event immediately, then forward broadcast events.
    let connected = SseEvent::Connected {
        server_time: Utc::now(),
    };

    let rx = tx.subscribe();
    let initial = stream::once(future::ready(Ok::<_, Infallible>(event_to_sse(&connected))));
    let stream: BoxStream<'static, Result<Event, Infallible>> = initial
        .chain(BroadcastStream::new(rx).map(|result| {
            Ok(match result {
                Ok(event) => event_to_sse(&event),
                Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "SSE client lagged behind broadcast stream");
                    event_to_sse(&SseEvent::StreamLagged { skipped })
                }
            })
        }))
        .boxed();

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Convert an [`SseEvent`] into an SSE [`Event`], setting the event type per
/// variant so clients can subscribe selectively.
///
/// Serialization should never fail for these well-typed enums; if it does,
/// the client receives a generic fallback event AND the failure is logged
/// so the operator notices (no silent swallowing).
fn event_to_sse(event: &SseEvent) -> Event {
    let name = match event {
        SseEvent::Connected { .. } => "connected",
        SseEvent::SearchResult { .. } => "search_result",
        SseEvent::StreamLagged { .. } => "stream_lagged",
    };
    let json = serde_json::to_string(event).unwrap_or_else(|err| {
        tracing::error!(
            error = %err,
            event_name = %name,
            "failed to serialize SseEvent — emitting fallback error payload"
        );
        r#"{"type":"error"}"#.to_owned()
    });
    Event::default().event(name).data(json)
}
