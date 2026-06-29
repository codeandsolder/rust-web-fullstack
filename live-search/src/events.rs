//! SSE event types for the live-search stream.
//!
//! Three event variants are defined:
//! - [`SseEvent::Connected`] — sent once when an SSE client first connects.
//! - [`SseEvent::SearchResult`] — a new search-result row inserted via
//!   `PostgreSQL` `NOTIFY`.
//! - [`SseEvent::StreamLagged`] — signals that the local broadcast channel
//!   dropped messages because the consumer fell behind.

use serde::{Deserialize, Serialize};

/// Events sent over the live-search SSE stream.
///
/// This enum is `#[non_exhaustive]` to permit future event types without
/// breaking downstream pattern matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum SseEvent {
    Connected,
    SearchResult {
        title: String,
        url: String,
        snippet: String,
    },
    StreamLagged {
        skipped: u64,
    },
}
