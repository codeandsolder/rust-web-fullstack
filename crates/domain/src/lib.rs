//! Shared domain types for the rust-web-fullstack workspace.
//!
//! This crate contains pure data types with no framework dependencies
//! (no `sqlx`, `leptos`, `axum`, etc.). It is the single source of truth
//! for domain model definitions consumed across workspace crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A search result as stored in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[must_use]
pub struct SearchResult {
    pub id: Uuid,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub created_at: DateTime<Utc>,
}
