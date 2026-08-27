//! Refresh-token storage backed by `PostgreSQL`.
//!
//! Raw refresh tokens are never persisted: the database stores only a SHA-256
//! digest. Tokens rotate atomically and remain linked by `family_id` and
//! `replaced_by`, allowing replay detection to revoke the active family.

use rwf_domain::UserId;
use sqlx::PgPool;
use sqlx::types::chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RotationResult {
    pub new_raw_token: String,
    pub subject: UserId,
}

pub const REFRESH_TOKEN_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Error)]
pub enum RefreshError {
    #[error("OS RNG failure: {0}")]
    OsRng(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid subject UUID in database")]
    UserId(#[from] rwf_domain::UserIdError),
    #[error("refresh-token TTL must be positive")]
    InvalidTtl,
}

#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub jti: Uuid,
    pub subject: UserId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Generate a fresh 256-bit opaque refresh token and its database JTI.
///
/// # Errors
/// Returns [`RefreshError::OsRng`] if the operating-system RNG fails.
pub fn generate_raw_refresh_token() -> Result<(String, Uuid), RefreshError> {
    let jti = Uuid::new_v4();
    let mut bytes = [0_u8; 32];
    aws_lc_rs::rand::fill(&mut bytes).map_err(|e| RefreshError::OsRng(e.to_string()))?;
    let raw = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes);
    Ok((raw, jti))
}

#[must_use]
pub fn hash_refresh_token(raw: &str) -> [u8; 32] {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, raw.as_bytes());
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// Rotate a refresh token inside one transaction.
///
/// Replaying any known inactive token revokes the rest of its family. On a
/// successful rotation the old row records both `revoked_at` and the JTI of
/// its replacement; the new row carries the same family id.
///
/// # Errors
/// Returns a DB/RNG/subject error or [`RefreshError::InvalidTtl`].
pub async fn rotate(
    pool: &PgPool,
    raw_token: &str,
    now: DateTime<Utc>,
    ttl_secs: i64,
) -> Result<Option<RotationResult>, RefreshError> {
    if ttl_secs <= 0 {
        return Err(RefreshError::InvalidTtl);
    }

    let hashed = hash_refresh_token(raw_token);
    let mut tx = pool.begin().await?;

    let active = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "SELECT jti, family_id, subject \
         FROM refresh_tokens \
         WHERE hashed_token = $1 \
           AND revoked_at IS NULL \
           AND expires_at > $2 \
         FOR UPDATE",
    )
    .bind(hashed)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((old_jti, family_id, subject)) = active else {
        let any = sqlx::query_as::<_, (Uuid,)>(
            "SELECT family_id FROM refresh_tokens WHERE hashed_token = $1 LIMIT 1",
        )
        .bind(hashed)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some((stolen_family,)) = any {
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
                "refresh-token replay detected; entire family revoked"
            );
        }
        tx.commit().await?;
        return Ok(None);
    };

    let subject = UserId::try_from(subject)?;

    // Generate the replacement before updating the old row so the chain is
    // persisted atomically. If RNG fails the transaction is rolled back.
    let (new_raw, new_jti) = generate_raw_refresh_token()?;
    let new_expires_at = now + chrono::Duration::seconds(ttl_secs);
    let new_hashed = hash_refresh_token(&new_raw).to_vec();

    sqlx::query(
        "UPDATE refresh_tokens \
         SET revoked_at = $1, replaced_by = $2 \
         WHERE jti = $3",
    )
    .bind(now)
    .bind(new_jti)
    .bind(old_jti)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO refresh_tokens \
         (jti, family_id, subject, hashed_token, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
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

/// Lookup helper used by tests and admin tooling.
///
/// # Errors
/// Returns a database error or an invalid stored user-id error.
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
        "SELECT jti, subject, created_at, expires_at, revoked_at \
         FROM refresh_tokens WHERE jti = $1",
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
        assert_eq!(hash_refresh_token("hello"), hash_refresh_token("hello"));
        assert_ne!(hash_refresh_token("hello"), hash_refresh_token("hellp"));
    }

    #[test]
    #[expect(
        clippy::panic,
        reason = "RNG failure on a test host indicates a broken environment"
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
        assert_eq!(a.len(), 43);
        assert_eq!(ja.get_version_num(), 4);
        assert_eq!(jb.get_version_num(), 4);
    }
}
