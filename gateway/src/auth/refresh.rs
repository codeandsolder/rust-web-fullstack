//! Refresh-token storage backed by `PostgreSQL`.
//!
//! Refresh tokens are issued alongside access tokens at login. Each
//! refresh token is a 32-byte cryptographically random secret,
//! base64url-encoded for transport, and stored as a SHA-256 hash in
//! the `refresh_tokens.hashed_token` column. The DB only ever holds
//! the hash; the raw token is returned to the client and never
//! persisted.
//!
//! ## Rotation & family revocation
//!
//! Every refresh token belongs to a `family_id`. The first row in a
//! family uses its own `jti` as the family id (see
//! [`crate::auth::handlers::login_handler`]). Successive rotations
//! reuse the same family id and chain through the `replaced_by`
//! column.
//!
//! A successful rotation atomically:
//! 1. Looks up the supplied token (must be unrevoked, unexpired).
//! 2. Marks it revoked.
//! 3. Inserts a new row with the **same `family_id`** and a new `jti`.
//!
//! If the supplied token is already revoked or unknown, [`rotate`]
//! treats this as a replay signal: it revokes every still-active
//! token in the family before returning `Ok(None)`. This is what
//! stops an attacker who stole one rotated token from continuing to
//! use it.

use rwf_domain::UserId;
use sqlx::PgPool;
use sqlx::types::chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

/// The result of a successful token rotation.
#[derive(Debug, Clone)]
pub struct RotationResult {
    /// The newly issued raw refresh token (base64url-encoded, 43 chars).
    pub new_raw_token: String,
    /// The subject (user) that the rotated token belongs to.
    pub subject: UserId,
}

/// Default lifetime for newly issued refresh tokens.
///
/// The constant is the historical 30-day default; production deployments
/// override this through `RWF_GATEWAY__REFRESH_TOKEN_TTL_SECS` (see
/// `rwf_config::GatewayConfig::refresh_token_ttl_secs`). Tests that don't
/// load `rwf-config` get this value as the floor.
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
    /// The subject UUID stored in the database was invalid (e.g. nil).
    #[error("invalid subject UUID in database")]
    UserId(#[from] rwf_domain::UserIdError),
}

/// One row of the `refresh_tokens` table, hydrated from a query.
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    /// Primary key (`UUIDv4` generated server-side).
    pub jti: Uuid,
    /// Subject (typically the user identifier).
    pub subject: UserId,
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

/// Atomically revoke the rotated token and insert a new refresh token
/// in the same family. Returns the new raw token and subject on
/// success. Returns `Ok(None)` if the input was already revoked,
/// expired, or unknown — the caller maps this to 401 to indicate the
/// credential chain is broken.
///
/// **Replay defence**: if the supplied token is in the DB at all
/// (even revoked), every still-active row in the same `family_id`
/// is revoked in the same transaction before returning `Ok(None)`.
/// This is what detects token theft: an attacker who replays a
/// rotated token causes the legitimate user's chain to be killed,
/// which the client notices on their next successful refresh.
///
/// `ttl_secs` controls the lifetime of the freshly-issued token;
/// pass `cfg.gateway.refresh_token_ttl_secs as i64` for production
/// behaviour.
///
/// # Errors
/// Returns [`RefreshError::OsRng`] when entropy cannot be fetched, or
/// [`RefreshError::Sqlx`] for DB failures.
pub async fn rotate(
    pool: &PgPool,
    raw_token: &str,
    now: DateTime<Utc>,
    ttl_secs: i64,
) -> Result<Option<RotationResult>, RefreshError> {
    let hashed = hash_refresh_token(raw_token);
    let mut tx = pool.begin().await?;

    // Try to find an active (unrevoked, unexpired) row for this hash.
    let active = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "
        SELECT jti, family_id, subject
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

    let Some((old_jti, family_id, subject)) = active else {
        // Either the token never existed, or it's revoked/expired.
        // If it ever existed (any state), treat the request as a
        // replay signal and revoke the whole family. Otherwise the
        // rotation request is simply unknown — no revocation needed.
        let any = sqlx::query_as::<_, (Option<Uuid>,)>(
            "SELECT family_id FROM refresh_tokens WHERE hashed_token = $1 LIMIT 1",
        )
        .bind(hashed)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((Some(stolen_family),)) = any {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at = $1 \
                 WHERE family_id = $2 AND revoked_at IS NULL",
            )
            .bind(now)
            .bind(stolen_family)
            .execute(&mut *tx)
            .await?;
            tracing::warn!(
                family_id = %stolen_family,
                "refresh-token replay detected; entire family revoked",
            );
        }
        tx.commit().await?;
        return Ok(None);
    };

    let subject = UserId::try_from(subject)?;

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

    // Insert a brand-new refresh token in the same family.
    let (new_raw, new_jti) = generate_raw_refresh_token()?;
    let new_expires_at = now + chrono::Duration::seconds(ttl_secs);
    let new_hashed = hash_refresh_token(&new_raw).to_vec();
    sqlx::query(
        "
        INSERT INTO refresh_tokens (jti, family_id, subject, hashed_token, expires_at, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        ",
    )
    .bind(new_jti)
    .bind(family_id)
    .bind(Uuid::from(subject))
    .bind(new_hashed)
    .bind(new_expires_at)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(RotationResult {
        new_raw_token: new_raw,
        subject,
    }))
}

/// Lookup helper used by tests and admin tooling. Not used by the
/// public refresh path; kept available for diagnostics.
///
/// # Errors
/// Returns [`RefreshError::Sqlx`] for DB failures or
/// [`RefreshError::UserId`] if the stored subject is invalid.
#[allow(dead_code)]
pub async fn find_by_jti(
    pool: &PgPool,
    jti: Uuid,
) -> Result<Option<RefreshTokenRecord>, RefreshError> {
    let row = sqlx::query_as::<
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
    .await?;

    match row {
        Some((jti, subject, created_at, expires_at, revoked_at)) => {
            let subject = UserId::try_from(subject)?;
            Ok(Some(RefreshTokenRecord {
                jti,
                subject,
                created_at,
                expires_at,
                revoked_at,
            }))
        }
        None => Ok(None),
    }
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
