use serde::{Deserialize, Serialize};

/// Events sent over the live-search SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
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
