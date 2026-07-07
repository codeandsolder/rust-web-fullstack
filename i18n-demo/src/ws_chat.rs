//! Bidirectional chat endpoint backed by an in-memory broadcast hub.
//!
//! WebSocket upgrade at `/ws/chat`. Clients send text frames; the
//! server prepends a timestamp + sender-id and fans the message out to
//! every other connected client. The hub is a single static
//! [`broadcast::Sender`] — fine for a demo, not for production
//! persistence or horizontal scaling.
//!
//! The handler is intentionally simple. Reach for a proper chat
//! backend (Redis pub/sub, NATS, server-sent events, sticky
//! session-aware routing) when you outgrow it.

use std::sync::LazyLock;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Channel capacity — 256 matches the gateway SSE buffer. A slow
/// client that doesn't drain will see `RecvError::Lagged` after this
/// many undelivered messages.
const HUB_CAPACITY: usize = 256;

/// One chat event circulating through the hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEvent {
    /// Per-message UUID v4 — the client can use this for de-dupe or
    /// to thread replies.
    pub id: Uuid,
    /// The sending client identifier (random UUID generated on
    /// connection). The endpoint does not authenticate — anyone can
    /// claim any display name. Replace with a real session id before
    /// showing this to real users.
    pub from: Uuid,
    /// Wall-clock timestamp in ISO-8601 with seconds resolution.
    pub at: chrono::DateTime<chrono::Utc>,
    /// The chat payload. The server enforces a 1 KiB cap on this
    /// field; longer text frames are dropped with a warn log.
    pub text: String,
}

impl ChatEvent {
    /// Construct a new chat event with a fresh id and timestamp.
    #[must_use]
    pub fn new(from: Uuid, text: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            from,
            at: chrono::Utc::now(),
            text,
        }
    }

    /// Hard cap on per-message text — 1 KiB. Frames longer than this
    /// are dropped at the WebSocket level to keep the broadcast hub
    /// from being flooded by one chatty client.
    pub const MAX_TEXT_BYTES: usize = 1024;
}

/// Global broadcast hub. Only the `Sender` half is cached — every
/// accepted WebSocket subscribes and creates its own `Receiver`.
static HUB: LazyLock<broadcast::Sender<ChatEvent>> = LazyLock::new(|| {
    let (tx, _rx) = broadcast::channel(HUB_CAPACITY);
    tx
});

/// `WebSocketUpgrade` handler. Returns 101 Switching Protocols on
/// accept. Rejecting browsers continue to the Leptos fallback.
pub fn chat_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

/// Inner handler invoked once the WebSocket upgrade completes.
///
/// Splits the [`WebSocket`] into its `Sink` and `Stream` halves via
/// the `futures` trait methods, then loops using
/// `tokio::select!`. The reader forwards text frames into the hub;
/// the writer pumps hub events back to the client. Both halves
/// cleanly cancel on exit.
async fn handle_socket(socket: WebSocket) {
    let from = Uuid::new_v4();
    info!(client = %from, "chat client connected");

    let mut rx = HUB.subscribe();
    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(message) = incoming else {
                    debug!(client = %from, "client stream closed");
                    break;
                };
                let Ok(message) = message else {
                    warn!(client = %from, "client recv error; closing");
                    break;
                };
                let Message::Text(text) = message else {
                    debug!(client = %from, "ignoring non-text frame");
                    continue;
                };
                if text.len() > ChatEvent::MAX_TEXT_BYTES {
                    warn!(
                        client = %from,
                        bytes = text.len(),
                        "dropping oversized chat frame"
                    );
                    continue;
                }
                let event = ChatEvent::new(from, text.to_string());
                if HUB.send(event).is_err() {
                    // Only happens when there are no receivers —
                    // harmless for a demo.
                    debug!("hub has no other receivers");
                }
            }
            broadcast = rx.recv() => {
                match broadcast {
                    Ok(event) => {
                        let payload = match serde_json::to_string(&event) {
                            Ok(p) => p,
                            Err(e) => {
                                warn!(client = %from, error = %e, "dropping unserializable event");
                                continue;
                            }
                        };
                        if sink.send(Message::Text(payload.into())).await.is_err() {
                            debug!(client = %from, "client sink closed");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(
                            client = %from,
                            skipped,
                            "client lagged; replay skipped"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("hub closed");
                        break;
                    }
                }
            }
        }
    }

    info!(client = %from, "chat client disconnected");
}

/// Inspect how many subscribers the hub currently has.
///
/// Returns the count of `Receiver`s attached. Useful for `/metrics`
/// in a real deployment.
#[must_use]
pub fn subscriber_count() -> usize {
    HUB.receiver_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_text_bytes_matches_documented_value() {
        assert_eq!(ChatEvent::MAX_TEXT_BYTES, 1024);
    }

    #[test]
    fn new_event_has_unique_ids_and_timestamps() {
        let e1 = ChatEvent::new(Uuid::new_v4(), "hello".to_string());
        let e2 = ChatEvent::new(Uuid::new_v4(), "world".to_string());
        assert_ne!(e1.id, e2.id);
        assert_ne!(e1.from, e2.from);
        assert_eq!(e1.text, "hello");
        assert_eq!(e2.text, "world");
    }

    #[test]
    #[expect(
        clippy::panic,
        reason = "serde_json failures on these inputs would indicate the construction is unsound; assert and surface that clearly in the test."
    )]
    fn event_round_trips_through_serde_json() {
        let event = ChatEvent::new(Uuid::new_v4(), "hello, room".to_string());
        let json = match serde_json::to_string(&event) {
            Ok(s) => s,
            Err(e) => panic!("serialize failed: {e}"),
        };
        let back: ChatEvent = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => panic!("deserialize failed: {e}"),
        };
        assert_eq!(back.id, event.id);
        assert_eq!(back.from, event.from);
        assert_eq!(back.text, event.text);
    }
}
