//! Refresh-token storage backed by `PostgreSQL`.
//!
//! Refresh tokens are issued alongside access tokens at login. Each
//! refresh token is a 32-byte cryptographically random secret,
//! base64url-encoded for transport, and stored as a SHA-256 hash in
//! the `refresh_tokens.hashed_token` column. The DB only ever holds
//! the hash; the raw token is returned to the client and never
//! persisted.
//!
//! Rotation semantics: every [`rotate`] call atomically revokes the
//! old token (`UPDATE ... SET revoked_at = NOW()`) and inserts a new
//! row in the same transaction. Re-using an already rotated token
//! returns `Ok(None)` which the handler maps to 401, signalling that
//! the credential may have been stolen.

use sqlx::PgPool;
use sqlx::types::chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

/// Default lifetime for newly issued refresh tokens (30 days).
pub const REFRESH_TOKEN_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;

/// Errors that can occur in refresh-token DB operations.
#[derive(Debug, Error)]
pub enum RefreshError {
    /// The operating system refused to provide entropy. Effectively
    /// impossible on Linux/macOS/Windows but treated as a hard error
    /// so we never silently hand out a weak token.
    #[error("OS RNG failure: {0}")]
    OsRng(String),
    /// Underlying `sqlx` error.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// One row of the `refresh_tokens` table, hydrated from a query.
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    /// Primary key (`UUIDv4` generated server-side).
    pub jti: Uuid,
    /// Subject (typically the user identifier).
    pub subject: Uuid,
    /// `created_at` from Postgres.
    pub created_at: DateTime<Utc>,
    /// `expires_at` from Postgres.
    pub expires_at: DateTime<Utc>,
    /// Optional revocation timestamp.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Generate a fresh refresh token (32 random bytes, base64url-encoded).
///
/// Returns `(raw_token, jti)` where `raw_token` is what the client
/// stores and `jti` is the primary key we will insert under.
///
/// # Errors
/// Returns [`RefreshError::OsRng`] if the operating-system RNG
/// refuses to provide entropy.
pub fn generate_raw_refresh_token() -> Result<(String, Uuid), RefreshError> {
    let jti = Uuid::new_v4();
    let mut bytes = [0_u8; 32];
    aws_lc_rs::rand::fill(&mut bytes).map_err(|e| RefreshError::OsRng(e.to_string()))?;
    let raw = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes);
    Ok((raw, jti))
}

/// Hash a raw refresh token with SHA-256, returning the 32-byte digest.
#[must_use]
pub fn hash_refresh_token(raw: &str) -> [u8; 32] {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, raw.as_bytes());
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// Persist a freshly issued refresh token.
///
/// The caller hands back the raw token to the client and stores only
/// the SHA-256 hash. If `subject` is not a `UUIDv4` the DB will reject
/// the insert — there is no implicit string↔UUID coercion.
///
/// # Errors
/// Returns [`RefreshError::OsRng`] when entropy cannot be fetched, or
/// [`RefreshError::Sqlx`] for DB failures.
#[allow(dead_code)]
pub async fn insert(
    pool: &PgPool,
    subject: Uuid,
    raw_token: &str,
    now: DateTime<Utc>,
) -> Result<Uuid, RefreshError> {
    let (raw, jti) = generate_raw_refresh_token()?;
    let _ = raw; // `raw_token` is the canonical value passed in.
    let hashed: Vec<u8> = hash_refresh_token(raw_token).to_vec();
    let expires_at = now + chrono::Duration::seconds(REFRESH_TOKEN_TTL_SECONDS);
    sqlx::query(
        "
        INSERT INTO refresh_tokens (jti, subject, hashed_token, expires_at, created_at)
        VALUES ($1, $2, $3, $4, $5)
        ",
    )
    .bind(jti)
    .bind(subject)
    .bind(&hashed)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(jti)
}

/// Atomically revoke the rotated token and insert a new refresh token
/// whose subject matches the rotated one. Returns the new raw token
/// on success. Returns `Ok(None)` if the input was already revoked or
/// expired — the caller maps this to 401 to indicate the credential
/// chain is broken.
///
/// # Errors
/// Returns [`RefreshError::OsRng`] when entropy cannot be fetched, or
/// [`RefreshError::Sqlx`] for DB failures.
pub async fn rotate(
    pool: &PgPool,
    raw_token: &str,
    now: DateTime<Utc>,
) -> Result<Option<String>, RefreshError> {
    let hashed = hash_refresh_token(raw_token);
    let mut tx = pool.begin().await?;

    let record = sqlx::query_as::<_, (Uuid, Uuid)>(
        "
        SELECT jti, subject
        FROM refresh_tokens
        WHERE hashed_token = $1
          AND revoked_at IS NULL
          AND expires_at > $2
        FOR UPDATE
        ",
    )
    .bind(hashed)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((old_jti, subject)) = record else {
        tx.rollback().await?;
        return Ok(None);
    };

    // Mark the old token revoked.
    sqlx::query(
        "
        UPDATE refresh_tokens
        SET revoked_at = $1
        WHERE jti = $2
        ",
    )
    .bind(now)
    .bind(old_jti)
    .execute(&mut *tx)
    .await?;

    // Insert a brand-new refresh token with the same subject.
    let (new_raw, new_jti) = generate_raw_refresh_token()?;
    let new_expires_at = now + chrono::Duration::seconds(REFRESH_TOKEN_TTL_SECONDS);
    let new_hashed = hash_refresh_token(&new_raw).to_vec();
    sqlx::query(
        "
        INSERT INTO refresh_tokens (jti, subject, hashed_token, expires_at, created_at)
        VALUES ($1, $2, $3, $4, $5)
        ",
    )
    .bind(new_jti)
    .bind(subject)
    .bind(new_hashed)
    .bind(new_expires_at)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(new_raw))
}

/// Lookup helper used by tests and admin tooling. Not used by the
/// public refresh path; kept available for diagnostics.
///
/// # Errors
/// Returns [`RefreshError::Sqlx`] for DB failures.
#[allow(dead_code)]
pub async fn find_by_jti(
    pool: &PgPool,
    jti: Uuid,
) -> Result<Option<RefreshTokenRecord>, RefreshError> {
    sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            DateTime<Utc>,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
        ),
    >(
        "
        SELECT jti, subject, created_at, expires_at, revoked_at
        FROM refresh_tokens
        WHERE jti = $1
        ",
    )
    .bind(jti)
    .fetch_optional(pool)
    .await
    .map(|row| {
        row.map(
            |(jti, subject, created_at, expires_at, revoked_at)| RefreshTokenRecord {
                jti,
                subject,
                created_at,
                expires_at,
                revoked_at,
            },
        )
    })
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_unique() {
        // Same input → same hash.
        assert_eq!(hash_refresh_token("hello"), hash_refresh_token("hello"),);
        // Different input → different hash.
        assert_ne!(hash_refresh_token("hello"), hash_refresh_token("hellp"),);
    }

    #[test]
    #[expect(
        clippy::panic,
        reason = "RNG failure on a test host indicates a broken environment; nothing we can do in test code."
    )]
    fn generator_returns_unique_tokens_and_valid_jtis() {
        let (a, ja) = match generate_raw_refresh_token() {
            Ok(v) => v,
            Err(e) => panic!("OS RNG unavailable on test host: {e}"),
        };
        let (b, jb) = match generate_raw_refresh_token() {
            Ok(v) => v,
            Err(e) => panic!("OS RNG unavailable on test host: {e}"),
        };
        assert_ne!(a, b);
        assert_ne!(ja, jb);
        // base64url-no-pad of 32 bytes is exactly 43 chars.
        assert_eq!(a.len(), 43);
        assert!(ja.get_version_num() == 4);
        assert!(jb.get_version_num() == 4);
    }
}
