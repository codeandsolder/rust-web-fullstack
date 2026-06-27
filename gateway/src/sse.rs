use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use serde_json::Value;
use std::convert::Infallible;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::gateway::GatewayState;

// ---------------------------------------------------------------------------
// Event model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum GatewayEvent {
    ServiceStarted(&'static str),
    ServiceStopped(&'static str),
    HealthChanged(&'static str, String),
    Custom(&'static str, Value),
}

impl From<GatewayEvent> for Event {
    fn from(event: GatewayEvent) -> Self {
        match event {
            GatewayEvent::ServiceStarted(name) => {
                Self::default().event("service_started").data(name)
            }
            GatewayEvent::ServiceStopped(name) => {
                Self::default().event("service_stopped").data(name)
            }
            GatewayEvent::HealthChanged(name, status) => Self::default()
                .event("health_changed")
                .data(format!("{name}: {status}")),
            GatewayEvent::Custom(typ, data) => Self::default().event(typ).data(data.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Publishing helper
// ---------------------------------------------------------------------------

/// Send an event on the shared broadcast channel.  Receivers that are too
/// slow are reported by the stream wrapper in the SSE handler.
pub fn publish_event(tx: &broadcast::Sender<GatewayEvent>, event: GatewayEvent) {
    if let Err(e) = tx.send(event) {
        tracing::debug!("gateway event had no SSE receivers: {e}");
    }
}

// ---------------------------------------------------------------------------
// SSE handler
// ---------------------------------------------------------------------------

/// SSE endpoint that streams all [`GatewayEvent`]s to connected clients.
pub async fn sse_handler(
    State(state): State<GatewayState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => Some(Ok(Event::from(event))),
        Err(BroadcastStreamRecvError::Lagged(n)) => {
            tracing::warn!("SSE client lagged by {n} messages");
            None
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
