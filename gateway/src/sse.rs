//! Server-Sent Events (SSE) streaming for gateway events.
//!
//! Events are published via a [`broadcast::Sender`] and streamed to connected
//! clients through an SSE endpoint.  The [`BroadcastStream`] wrapper converts
//! channel closure into end-of-stream (`None`); only the `Lagged` error is
//! surfaced (and logged) so slow consumers gracefully drop messages.

use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::gateway::GatewayState;

// ---------------------------------------------------------------------------
// Event model
// ---------------------------------------------------------------------------

/// Gateway event published over the SSE channel.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum GatewayEvent {
    /// A service has started.
    ServiceStarted(&'static str),
    /// A service has stopped.
    ServiceStopped(&'static str),
    /// A service's health status changed.
    HealthChanged(&'static str, Arc<str>),
    /// A custom event with a type tag and JSON payload.
    Custom(&'static str, Value),
}

/// Serializable view of a [`GatewayEvent`] for SSE JSON serialization.
///
/// This avoids allocating intermediate `String` buffers in the hot (SSE)
/// path by borrowing directly from the [`GatewayEvent`] variants.
#[derive(Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
enum GatewayEventView<'a> {
    ServiceStarted { name: &'a str },
    ServiceStopped { name: &'a str },
    HealthChanged { name: &'a str, status: &'a str },
}

/// Build an SSE `Event` with the given type and JSON payload, falling back to
/// a safe placeholder on serialization failure (which never happens for our
/// well-typed variants but is logged for visibility).
fn build_event(event_name: &'static str, data: impl Serialize) -> Event {
    Event::default()
        .event(event_name)
        .json_data(&data)
        .unwrap_or_else(|err| {
            tracing::error!(
                error = %err,
                event_name = %event_name,
                "failed to serialize gateway event payload"
            );
            Event::default()
                .event(event_name)
                .data("serialization error")
        })
}

impl From<GatewayEvent> for Event {
    fn from(event: GatewayEvent) -> Self {
        match event {
            GatewayEvent::ServiceStarted(name) => {
                build_event("service_started", GatewayEventView::ServiceStarted { name })
            }
            GatewayEvent::ServiceStopped(name) => {
                build_event("service_stopped", GatewayEventView::ServiceStopped { name })
            }
            GatewayEvent::HealthChanged(name, status) => build_event(
                "health_changed",
                GatewayEventView::HealthChanged {
                    name,
                    status: status.as_ref(),
                },
            ),
            GatewayEvent::Custom(typ, data) => build_event(typ, data),
        }
    }
}

// ---------------------------------------------------------------------------
// Publishing helper
// ---------------------------------------------------------------------------

/// Send an event on the shared broadcast channel.
///
/// Receivers that are too slow are reported by the stream wrapper in the
/// SSE handler.
pub fn publish_event(tx: &broadcast::Sender<GatewayEvent>, event: GatewayEvent) {
    if let Err(e) = tx.send(event) {
        tracing::debug!(error = %e, "gateway event had no SSE receivers");
    }
}

// ---------------------------------------------------------------------------
// SSE handler
// ---------------------------------------------------------------------------

/// SSE endpoint that streams all [`GatewayEvent`]s to connected clients.
///
/// The [`BroadcastStream`] wrapper natively converts channel closure into
/// end-of-stream (`Poll::Ready(None)`), so no explicit `Closed` handler is
/// needed here.  The only recoverable error is [`BroadcastStreamRecvError::Lagged`],
/// which is logged and filtered out.
pub async fn sse_handler(
    State(state): State<GatewayState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => Some(Ok(Event::from(event))),
        Err(BroadcastStreamRecvError::Lagged(n)) => {
            tracing::warn!(count = %n, "SSE client lagged");
            None
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
