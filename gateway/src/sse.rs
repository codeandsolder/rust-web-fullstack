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

impl From<GatewayEvent> for Event {
    fn from(event: GatewayEvent) -> Self {
        match event {
            GatewayEvent::ServiceStarted(name) => {
                let view = GatewayEventView::ServiceStarted { name };
                let event = Self::default().event("service_started");
                match event.json_data(&view) {
                    Ok(ev) => ev,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize GatewayEvent::ServiceStarted");
                        Self::default()
                            .event("service_started")
                            .data("serialization error")
                    }
                }
            }
            GatewayEvent::ServiceStopped(name) => {
                let view = GatewayEventView::ServiceStopped { name };
                let event = Self::default().event("service_stopped");
                match event.json_data(&view) {
                    Ok(ev) => ev,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize GatewayEvent::ServiceStopped");
                        Self::default()
                            .event("service_stopped")
                            .data("serialization error")
                    }
                }
            }
            GatewayEvent::HealthChanged(name, status) => {
                let view = GatewayEventView::HealthChanged {
                    name,
                    status: status.as_ref(),
                };
                let event = Self::default().event("health_changed");
                match event.json_data(&view) {
                    Ok(ev) => ev,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize GatewayEvent::HealthChanged");
                        Self::default()
                            .event("health_changed")
                            .data("serialization error")
                    }
                }
            }
            GatewayEvent::Custom(typ, data) => {
                let event = Self::default().event(typ);
                match event.json_data(&data) {
                    Ok(ev) => ev,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize GatewayEvent::Custom");
                        Self::default().event(typ).data("serialization error")
                    }
                }
            }
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
