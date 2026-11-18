//! Checked token expiration arithmetic shared by JWT and refresh-token paths.
//!
//! Chrono's `TimeDelta::seconds` and date arithmetic can panic for TTL values
//! that fit `i64` but exceed its representable duration or timestamp range.

use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;

/// A token lifetime that cannot be represented as a future expiration.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenExpiryError {
    #[error("token TTL must be positive")]
    NonPositiveTtl,
    #[error("token expiration is outside the representable date range")]
    OutOfRange,
}

/// Compute a future token expiration without panicking on any signed TTL.
///
/// # Errors
/// Returns [`TokenExpiryError::NonPositiveTtl`] for zero/negative lifetimes,
/// or [`TokenExpiryError::OutOfRange`] for unrepresentable durations or dates.
pub fn expires_at(now: DateTime<Utc>, ttl_secs: i64) -> Result<DateTime<Utc>, TokenExpiryError> {
    if ttl_secs <= 0 {
        return Err(TokenExpiryError::NonPositiveTtl);
    }
    let duration = TimeDelta::try_seconds(ttl_secs).ok_or(TokenExpiryError::OutOfRange)?;
    now.checked_add_signed(duration)
        .ok_or(TokenExpiryError::OutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_positive_expiration() -> Result<(), TokenExpiryError> {
        let now = DateTime::<Utc>::UNIX_EPOCH;
        assert_eq!(expires_at(now, 120)?.timestamp(), 120);
        Ok(())
    }

    #[test]
    fn rejects_nonpositive_lifetimes() {
        let now = DateTime::<Utc>::UNIX_EPOCH;
        for ttl in [i64::MIN, -1, 0] {
            assert_eq!(expires_at(now, ttl), Err(TokenExpiryError::NonPositiveTtl));
        }
    }

    #[test]
    fn rejects_duration_and_datetime_overflow() {
        let now = DateTime::<Utc>::UNIX_EPOCH;
        assert_eq!(expires_at(now, i64::MAX), Err(TokenExpiryError::OutOfRange));
        assert_eq!(
            expires_at(DateTime::<Utc>::MAX_UTC, 1),
            Err(TokenExpiryError::OutOfRange),
        );
    }
}
