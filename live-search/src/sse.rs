//! Server-Sent Events handler and broadcast channel setup.
//!
//! A single [`broadcast::Sender<SseEvent>`] is initialised at startup and
//! shared between the `PgListener` task (producer) and all SSE client
//! connections (consumers).  The [`sse_handler`] function is the axum route
//! handler that streams events to HTTP clients.

use std::convert::Infallible;
use std::sync::OnceLock;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use futures::StreamExt;
use futures::future;
use futures::stream::{self, BoxStream};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::events::SseEvent;

/// Global broadcast sender, initialized once on startup.
static BROADCAST: OnceLock<broadcast::Sender<SseEvent>> = OnceLock::new();

/// Error returned when the global broadcast sender cannot be initialized.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BroadcastInitError {
    /// The sender was already set earlier in the process lifetime.
    #[error("broadcast sender already initialized")]
    AlreadyInitialized,
}

/// Sets the global broadcast sender.
///
/// # Errors
/// Returns [`BroadcastInitError::AlreadyInitialized`] if startup tries to set
/// the sender more than once.
pub fn set_broadcast(tx: broadcast::Sender<SseEvent>) -> Result<(), BroadcastInitError> {
    BROADCAST
        .set(tx)
        .map_err(|_| BroadcastInitError::AlreadyInitialized)
}

/// Returns a reference to the global broadcast sender.
#[must_use]
pub fn get_broadcast() -> Option<&'static broadcast::Sender<SseEvent>> {
    BROADCAST.get()
}

/// Emit a `"configuration_error"` event when the broadcast sender is unset.
fn config_error_event() -> Event {
    Event::default()
        .event("configuration_error")
        .data(r#"{"type":"ConfigurationError","message":"broadcast sender is not initialized"}"#)
}

/// SSE handler: streams events from the broadcast channel to the client.
#[expect(
    clippy::unused_async,
    reason = "Axum 0.8 requires async fn for Handler trait"
)]
pub async fn sse_handler() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Emit a "Connected" event immediately, then forward broadcast events.
    let connected = SseEvent::Connected;

    let stream: BoxStream<'static, Result<Event, Infallible>> = get_broadcast().map_or_else(
        || {
            tracing::error!("SSE broadcast sender is not initialized");
            stream::once(future::ready(Ok(config_error_event()))).boxed()
        },
        |tx| {
            let rx = tx.subscribe();
            let initial =
                stream::once(future::ready(Ok::<_, Infallible>(event_to_sse(&connected))));
            initial
                .chain(BroadcastStream::new(rx).map(|result| {
                    Ok(match result {
                        Ok(event) => event_to_sse(&event),
                        Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                            tracing::warn!(skipped, "SSE client lagged behind broadcast stream");
                            event_to_sse(&SseEvent::StreamLagged { skipped })
                        }
                    })
                }))
                .boxed()
        },
    );

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
        SseEvent::Connected => "connected",
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
