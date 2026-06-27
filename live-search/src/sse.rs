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

/// SSE handler: streams events from the broadcast channel to the client.
///
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
            stream::once(future::ready(Ok(Event::default()
                .event("configuration_error")
                .data(
                    r#"{"type":"ConfigurationError","message":"broadcast sender is not initialized"}"#,
                ))))
            .boxed()
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

fn event_to_sse(event: &SseEvent) -> Event {
    match serde_json::to_string(event) {
        Ok(data) => Event::default().data(data),
        Err(e) => {
            tracing::error!("failed to serialize SSE event: {e}");
            Event::default()
                .event("serialization_error")
                .data(r#"{"type":"SerializationError"}"#)
        }
    }
}
