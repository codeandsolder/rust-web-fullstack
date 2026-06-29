//! SSE event types for the live-search stream.
//!
//! Three event variants are defined:
//! - [`SseEvent::Connected`] — sent once when an SSE client first connects.
//! - [`SseEvent::SearchResult`] — a new search-result row inserted via
//!   `PostgreSQL` `NOTIFY`.
//! - [`SseEvent::StreamLagged`] — signals that the local broadcast channel
//!   dropped messages because the consumer fell behind.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Serialize or deserialize an `Arc<str>` as a plain JSON string.
mod arc_str_serde {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Arc<str>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<str>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Arc::from(s))
    }
}

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
        /// The three String fields are stored as `Arc<str>` so that
        /// `broadcast::Sender::send` only bumps a ref-count per subscriber
        /// instead of performing a full heap allocation per field per
        /// receiver.
        #[serde(with = "arc_str_serde")]
        title: Arc<str>,
        #[serde(with = "arc_str_serde")]
        url: Arc<str>,
        #[serde(with = "arc_str_serde")]
        snippet: Arc<str>,
    },
    StreamLagged {
        skipped: u64,
    },
}
