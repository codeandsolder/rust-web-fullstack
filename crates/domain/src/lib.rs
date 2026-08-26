//! Shared domain types for the rust-web-fullstack workspace.
//!
//! This crate contains pure data types with no framework dependencies
//! (no `sqlx`, `leptos`, `axum`, etc.). It is the single source of truth
//! for domain model definitions consumed across workspace crates.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// A non-nil user identifier. Newtype wrapper around `Uuid` to prevent mixing
/// a `UserId` with a generic `Uuid`, `OrgId`, or other domain identifier.
///
/// Wire-format compatibility: serialises as a plain UUID string
/// (`#[serde(try_from = "Uuid", into = "Uuid")]`), so JWTs containing a
/// `Claims::sub: UserId` and DB rows containing a `subject: UserId` round-trip
/// through `serde_json` and `jsonwebtoken` without an object wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "Uuid", into = "Uuid")]
#[must_use = "a UserId should not be discarded"]
pub struct UserId(Uuid);

impl UserId {
    /// Return the inner `Uuid` by value.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl TryFrom<Uuid> for UserId {
    type Error = UserIdError;

    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        if value.is_nil() {
            Err(UserIdError::Nil)
        } else {
            Ok(Self(value))
        }
    }
}

impl From<UserId> for Uuid {
    fn from(value: UserId) -> Self {
        value.0
    }
}

impl AsRef<Uuid> for UserId {
    fn as_ref(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for UserId {
    type Err = UserIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(s).map_err(UserIdError::Parse)?;
        Self::try_from(uuid)
    }
}

/// Errors that can arise when constructing or parsing a [`UserId`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UserIdError {
    /// The UUID was the nil UUID — rejected because it is never a valid
    /// user identifier.
    #[error("user id must not be the nil UUID")]
    Nil,
    /// The input string was not a valid UUID.
    #[error("invalid UUID string: {0}")]
    Parse(#[source] uuid::Error),
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_nil_uuid() {
        assert!(UserId::try_from(Uuid::nil()).is_err());
    }

    #[test]
    fn serde_json_roundtrips_as_plain_uuid_string() -> Result<(), Box<dyn std::error::Error>> {
        let id = UserId::try_from(Uuid::from_u128(
            0x1234_5678_9ABC_DEF0_1234_5678_9ABC_DEF0,
        ))?;
        let json = serde_json::to_string(&id)?;
        assert_eq!(json, "\"12345678-9abc-def0-1234-56789abcdef0\"");

        let parsed: UserId = serde_json::from_str(&json)?;
        assert_eq!(parsed, id);
        Ok(())
    }

    #[test]
    fn from_str_rejects_nil_uuid() {
        assert!(UserId::from_str("00000000-0000-0000-0000-000000000000").is_err());
    }
}
