//! JSON assertion helpers, insta snapshot utilities, and proptest strategies.

/// Assert that two JSON values are structurally equal (ignoring field order).
/// Thin wrapper around `serde_json::Value`'s `PartialEq` implementation.
#[allow(
    dead_code,
    reason = "Available for test assertions; may be unused per-binary"
)]
#[must_use]
pub fn json_eq(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    left == right
}

#[cfg(test)]
mod proptests {
    use chrono::{Duration, Utc};
    use proptest::prelude::*;
    use uuid::Uuid;

    use live_search::db::SearchResult;

    /// Strategy that generates arbitrary `SearchResult` values using the real
    /// domain types (`Uuid`, `DateTime<Utc>`).
    fn arb_search_result() -> impl Strategy<Value = SearchResult> {
        (
            any::<u128>().prop_map(Uuid::from_u128),
            "[a-zA-Z0-9 ]{1,100}",
            "[a-zA-Z0-9:/._-]{1,200}",
            "[a-zA-Z0-9 ]{0,500}",
            (0i64..1_000_000_000i64).prop_map(|secs| Utc::now() - Duration::seconds(secs)),
        )
            .prop_map(|(id, title, url, snippet, created_at)| SearchResult {
                id,
                title,
                url,
                snippet,
                created_at,
            })
    }

    proptest! {
        /// SearchResult round-trips through serde_json losslessly.
        #[test]
        fn search_result_json_roundtrip(result in arb_search_result()) {
            let json = serde_json::to_value(&result)
                .expect("serialization must succeed");
            let deserialized: SearchResult =
                serde_json::from_value(json.clone())
                    .expect("deserialization must succeed");
            // Compare serialized forms since SearchResult doesn't impl PartialEq.
            let re_serialized = serde_json::to_value(&deserialized)
                .expect("re-serialization must succeed");
            prop_assert_eq!(json, re_serialized);
        }
    }
}
