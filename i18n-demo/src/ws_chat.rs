//! Bidirectional chat endpoint backed by an in-memory broadcast hub.
//!
//! WebSocket upgrade at `/ws/chat`. Browser requests are same-origin checked,
//! frame/message size limits are enforced by Axum before large payloads are
//! buffered, and each accepted text message is fanned out to every *other*
//! connected client. The hub is in-memory and intentionally demo-only.

use std::sync::LazyLock;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use uuid::Uuid;

const HUB_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEvent {
    pub id: Uuid,
    pub from: Uuid,
    /// UTC timestamp serialized by Chrono as RFC3339.
    pub at: chrono::DateTime<chrono::Utc>,
    pub text: String,
}

impl ChatEvent {
    #[must_use]
    pub fn new(from: Uuid, text: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            from,
            at: chrono::Utc::now(),
            text,
        }
    }

    /// Maximum accepted WebSocket message/frame size.
    pub const MAX_TEXT_BYTES: usize = 1024;
}

static HUB: LazyLock<broadcast::Sender<ChatEvent>> = LazyLock::new(|| {
    let (tx, _rx) = broadcast::channel(HUB_CAPACITY);
    tx
});

fn origin_matches_host(origin: &str, host: &str) -> bool {
    let Some(without_scheme) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = without_scheme.split('/').next().unwrap_or_default();
    !authority.is_empty() && authority.eq_ignore_ascii_case(host)
}

/// WebSocket upgrade handler.
///
/// Browser-originated upgrades must be same-origin. The protocol layer rejects
/// oversized frames/messages before application code receives them; the
/// in-loop length check remains as defense in depth.
pub async fn chat_handler(headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if let Some(origin_value) = headers.get(ORIGIN) {
        let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
            return StatusCode::FORBIDDEN.into_response();
        };
        let Ok(origin) = origin_value.to_str() else {
            return StatusCode::FORBIDDEN.into_response();
        };
        if !origin_matches_host(origin, host) {
            warn!(origin, host, "rejecting cross-origin websocket upgrade");
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    ws.max_message_size(ChatEvent::MAX_TEXT_BYTES)
        .max_frame_size(ChatEvent::MAX_TEXT_BYTES)
        .on_upgrade(handle_socket)
}

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
                    warn!(client = %from, bytes = text.len(), "dropping oversized chat message");
                    continue;
                }
                let event = ChatEvent::new(from, text.to_string());
                let _ = HUB.send(event);
            }
            broadcast = rx.recv() => {
                match broadcast {
                    Ok(event) => {
                        if event.from == from {
                            continue;
                        }
                        let payload = match serde_json::to_string(&event) {
                            Ok(payload) => payload,
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
                        warn!(client = %from, skipped, "client lagged; replay skipped");
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
    fn origin_validation_accepts_same_host_only() {
        assert!(origin_matches_host("http://localhost:3002", "localhost:3002"));
        assert!(origin_matches_host("https://example.com", "example.com"));
        assert!(!origin_matches_host("https://evil.example", "example.com"));
        assert!(!origin_matches_host("null", "example.com"));
    }

    #[test]
    fn new_event_has_unique_ids_and_payloads() {
        let e1 = ChatEvent::new(Uuid::new_v4(), "hello".to_string());
        let e2 = ChatEvent::new(Uuid::new_v4(), "world".to_string());
        assert_ne!(e1.id, e2.id);
        assert_ne!(e1.from, e2.from);
        assert_eq!(e1.text, "hello");
        assert_eq!(e2.text, "world");
    }

    #[test]
    #[expect(clippy::panic, reason = "serde failure would make this test fixture invalid")]
    fn event_round_trips_through_serde_json() {
        let event = ChatEvent::new(Uuid::new_v4(), "hello, room".to_string());
        let json = match serde_json::to_string(&event) {
            Ok(value) => value,
            Err(e) => panic!("serialize failed: {e}"),
        };
        let back: ChatEvent = match serde_json::from_str(&json) {
            Ok(value) => value,
            Err(e) => panic!("deserialize failed: {e}"),
        };
        assert_eq!(back.id, event.id);
        assert_eq!(back.from, event.from);
        assert_eq!(back.text, event.text);
    }
}
