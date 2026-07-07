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

#[cfg(test)]
mod tests {
    use crate::events::SseEvent;

    /// `serde_json::to_string(&SseEvent)` produces the JSON the SSE handler
    /// embeds in the `data:` field. We test the JSON shape directly because
    /// `Event` is opaque (no public field access for the event name or
    /// data string) and the public contract is "client subscribes via
    /// `addEventListener('event-name', …)` and parses the data as JSON".
    /// Verifying the JSON shape is the meaningful unit.
    #[expect(
        clippy::expect_used,
        reason = "test fixtures: chrono::from_timestamp(0,0) is infallible on a sound system clock, and serde_json of the canonical enum cannot fail"
    )]
    #[test]
    fn sse_event_json_shape_per_variant() {
        let epoch =
            chrono::DateTime::from_timestamp(0, 0).expect("unix epoch is always representable");
        let connected = SseEvent::Connected { server_time: epoch };
        let json = serde_json::to_string(&connected).expect("Connected serializes");
        assert!(
            json.contains("\"type\":\"Connected\""),
            "Connected must carry the `type` tag; got {json}"
        );
        assert!(
            json.contains("\"server_time\""),
            "Connected must carry server_time; got {json}"
        );

        let result = SseEvent::SearchResult {
            title: "t".into(),
            url: "u".into(),
            snippet: "s".into(),
        };
        let json = serde_json::to_string(&result).expect("SearchResult serializes");
        assert!(json.contains("\"type\":\"SearchResult\""));
        assert!(json.contains("\"title\":\"t\""));
        assert!(json.contains("\"url\":\"u\""));
        assert!(json.contains("\"snippet\":\"s\""));

        let lagged = SseEvent::StreamLagged { skipped: 42 };
        let json = serde_json::to_string(&lagged).expect("StreamLagged serializes");
        assert!(json.contains("\"type\":\"StreamLagged\""));
        assert!(json.contains("\"skipped\":42"));
    }

    /// The `connected` / `search_result` / `stream_lagged` event-name
    /// mapping is hard-coded in `event_to_sse` via the SSE `event:`
    /// field of the rendered wire format. Axum 0.8's `Event` struct is
    /// opaque (no `pub` accessors for the event name or data), so the
    /// end-to-end mapping is best verified at the integration-test
    /// layer (e2e-tests/tests/sse_test.rs), which subscribes via a
    /// real `EventSource` and asserts on the parsed event names.
    #[expect(
        clippy::expect_used,
        reason = "test fixture: chrono::from_timestamp(0,0) is infallible on a sound system clock"
    )]
    #[test]
    fn sse_event_variants_are_all_reachable() {
        // Compile-time check that all three variants still exist (regression
        // guard against accidental removal in a future refactor).
        let epoch =
            chrono::DateTime::from_timestamp(0, 0).expect("unix epoch is always representable");
        let _ = SseEvent::Connected { server_time: epoch };
        let _ = SseEvent::SearchResult {
            title: "t".into(),
            url: "u".into(),
            snippet: "s".into(),
        };
        let _ = SseEvent::StreamLagged { skipped: 1 };
    }
}
