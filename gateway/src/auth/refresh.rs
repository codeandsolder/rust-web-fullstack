//! Refresh-token storage backed by `PostgreSQL`.
//!
//! Raw refresh tokens are never persisted: the database stores only a SHA-256
//! digest. Tokens rotate atomically and remain linked by `family_id` and
//! `replaced_by`, allowing replay detection to revoke the active family.

use rwf_domain::UserId;
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use super::expiry::{self, TokenExpiryError};

#[derive(Clone)]
pub struct RotationResult {
    pub new_raw_token: String,
    pub subject: UserId,
}

impl std::fmt::Debug for RotationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RotationResult")
            .field("new_raw_token", &"<redacted>")
            .field("subject", &self.subject)
            .finish()
    }
}

/// A rotation whose database writes are not yet committed. Dropping this
/// value rolls them back (including the previous token's revocation).
#[must_use = "commit only after the access token is successfully issued"]
pub struct PreparedRotation {
    tx: Transaction<'static, Postgres>,
    new_raw_token: String,
    subject: UserId,
}

impl std::fmt::Debug for PreparedRotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedRotation")
            .field("new_raw_token", &"<redacted>")
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

impl PreparedRotation {
    pub const fn subject(&self) -> UserId {
        self.subject
    }

    /// Persist the rotation once the replacement access credential is ready.
    ///
    /// # Errors
    /// Returns a database error if the transaction cannot be committed.
    pub async fn commit(self) -> Result<RotationResult, sqlx::Error> {
        let Self {
            tx,
            new_raw_token,
            subject,
        } = self;
        tx.commit().await?;
        Ok(RotationResult {
            new_raw_token,
            subject,
        })
    }
}

#[derive(Debug, Error)]
pub enum RefreshError {
    #[error("OS RNG failure: {0}")]
    OsRng(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid subject UUID in database")]
    UserId(#[from] rwf_domain::UserIdError),
    #[error(transparent)]
    Expiry(#[from] TokenExpiryError),
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

/// Prepare a refresh-token rotation inside an open database transaction.
///
/// The caller must create the replacement access token before calling
/// [`PreparedRotation::commit`]. Dropping a prepared rotation on an issuance
/// failure rolls back both the previous token's revocation and the new insert.
/// Replaying any known inactive token still commits revocation of its family.
///
/// # Errors
/// Returns a DB/RNG/subject error or [`RefreshError::Expiry`] when the TTL
/// cannot yield a representable future date.
pub async fn prepare_rotation(
    pool: &PgPool,
    raw_token: &str,
    now: DateTime<Utc>,
    ttl_secs: i64,
) -> Result<Option<PreparedRotation>, RefreshError> {
    let new_expires_at = expiry::expires_at(now, ttl_secs)?;
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

    Ok(Some(PreparedRotation {
        tx,
        new_raw_token: new_raw,
        subject,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_result_debug_redacts_refresh_secret() -> Result<(), rwf_domain::UserIdError> {
        let result = RotationResult {
            new_raw_token: "unique-refresh-credential-secret".into(),
            subject: UserId::try_from(Uuid::new_v4())?,
        };
        let printed = format!("{result:?}");
        assert!(printed.contains("<redacted>"));
        assert!(!printed.contains("unique-refresh-credential-secret"));
        Ok(())
    }

    #[test]
    fn hash_is_deterministic_and_unique() {
        assert_eq!(hash_refresh_token("hello"), hash_refresh_token("hello"));
        assert_ne!(hash_refresh_token("hello"), hash_refresh_token("hellp"));
    }

    #[test]
    fn generator_returns_unique_tokens_and_valid_jtis() -> Result<(), RefreshError> {
        let (a, ja) = generate_raw_refresh_token()?;
        let (b, jb) = generate_raw_refresh_token()?;
        assert_ne!(a, b);
        assert_ne!(ja, jb);
        assert_eq!(a.len(), 43);
        assert_eq!(ja.get_version_num(), 4);
        assert_eq!(jb.get_version_num(), 4);
        Ok(())
    }
}
