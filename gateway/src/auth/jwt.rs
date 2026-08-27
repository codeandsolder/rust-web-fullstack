//! JWT creation and validation using `EdDSA` (Ed25519).
//!
//! # Key format
//!
//! The signing key is loaded from a PKCS#8 PEM string (the same format that
//! `openssl genpkey -algorithm ED25519` produces) and the
//! verifying key from an SPKI PEM string.
//!
//! # Thread safety
//!
//! PEM parsing on every call is a few microseconds.  For hot paths the caller
//! should cache the parsed [`jsonwebtoken::EncodingKey`] /
//! [`jsonwebtoken::DecodingKey`] objects.

use crate::settings::{JWT_AUD, JWT_ISS};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rwf_domain::UserId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AppError;

/// JWT claims payload.
///
/// Standard fields: `sub` (subject), `exp` (expiration), `iat` (issued-at),
/// `aud` (audience), `iss` (issuer), `jti` (JWT ID).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
#[must_use]
pub struct Claims {
    pub sub: UserId,
    pub exp: u64,
    pub iat: u64,
    pub aud: String,
    pub iss: String,
    pub jti: Uuid,
}

/// Convert a [`chrono::DateTime<Utc>`] to a `u64` Unix timestamp, returning an
/// [`AppError::Internal`] on overflow (e.g. dates before epoch).
fn unix_seconds(t: chrono::DateTime<Utc>) -> Result<u64, AppError> {
    u64::try_from(t.timestamp()).map_err(|e| AppError::internal("timestamp overflow", e))
}

/// Create a signed `EdDSA` JWT for the given `user_id`.
///
/// The token expires `ttl_secs` seconds from creation. Production callers
/// should pass a short value (e.g. 15 minutes) so that DB-backed refresh
/// tokens carry meaningful state. A 24-hour access token is not a "short-lived
/// access token".
///
/// The caller is expected to provide a cached [`EncodingKey`] (e.g. from
/// [`Settings::encoding_key`]).
///
/// A random `jti` is generated for every token.
///
/// # Errors
///
/// Returns [`AppError::Internal`] if encoding fails or `ttl_secs` overflows
/// the i64 range.
pub fn create_jwt(
    user_id: &UserId,
    encoding_key: &EncodingKey,
    ttl_secs: i64,
) -> Result<String, AppError> {
    let now = Utc::now();
    let exp = unix_seconds(now + chrono::Duration::seconds(ttl_secs))?;

    let claims = Claims {
        sub: *user_id,
        iat: unix_seconds(now)?,
        exp,
        aud: JWT_AUD.to_string(),
        iss: JWT_ISS.to_string(),
        jti: Uuid::new_v4(),
    };

    let header = Header::new(Algorithm::EdDSA);
    encode(&header, &claims, encoding_key).map_err(|e| AppError::internal("JWT encoding", e))
}

/// Validate an `EdDSA` JWT and return its [`Claims`].
///
/// # Errors
///
/// Returns [`AppError::TokenExpired`] if the token has expired,
/// [`AppError::InvalidSignature`] if the signature is invalid, or
/// [`AppError::Jwt`] for other decoding errors.
pub fn validate_jwt(token: &str, decoding_key: &DecodingKey) -> Result<Claims, AppError> {
    use jsonwebtoken::errors::ErrorKind;

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[JWT_ISS]);
    validation.set_audience(&[JWT_AUD]);

    decode::<Claims>(token, decoding_key, &validation)
        .map(|data| data.claims)
        .map_err(|e| match e.kind() {
            ErrorKind::ExpiredSignature => AppError::TokenExpired(e),
            ErrorKind::InvalidSignature => AppError::InvalidSignature(e),
            _ => AppError::Jwt(e),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pem::{ed25519_pkcs8_der, ed25519_spki_der, pem_encode};
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

    /// Deterministic keypair from a fixed seed so tests are reproducible.
    const TEST_SEED: [u8; 32] = [1u8; 32];

    fn dev_keypair_pems() -> anyhow::Result<(String, String)> {
        let key_pair = Ed25519KeyPair::from_seed_unchecked(&TEST_SEED)?;
        let public_key = key_pair.public_key().as_ref().to_vec();
        let private_pem = pem_encode("PRIVATE KEY", &ed25519_pkcs8_der(&TEST_SEED));
        let public_pem = pem_encode("PUBLIC KEY", &ed25519_spki_der(&public_key));
        Ok((private_pem, public_pem))
    }

    #[test]
    fn sign_and_verify_roundtrip() -> anyhow::Result<()> {
        let (private_pem, public_pem) = dev_keypair_pems()?;
        let encoding_key = EncodingKey::from_ed_pem(private_pem.as_bytes())?;
        let decoding_key = DecodingKey::from_ed_pem(public_pem.as_bytes())?;

        let user_id = UserId::try_from(Uuid::new_v4())?;
        let token = create_jwt(&user_id, &encoding_key, 60 * 60 * 24)?;

        let claims = validate_jwt(&token, &decoding_key)?;
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.iss, JWT_ISS);
        assert_eq!(claims.aud, JWT_AUD);
        assert_ne!(claims.iss, claims.aud);
        assert!(claims.exp > claims.iat);
        assert_ne!(claims.jti, Uuid::nil());
        Ok(())
    }

    #[test]
    fn rejects_wrong_key() -> anyhow::Result<()> {
        let (private_pem, _) = dev_keypair_pems()?;
        let encoding_key = EncodingKey::from_ed_pem(private_pem.as_bytes())?;

        let wrong_seed = [2u8; 32];
        let wrong_key_pair = Ed25519KeyPair::from_seed_unchecked(&wrong_seed)?;
        let wrong_public_pem = pem_encode(
            "PUBLIC KEY",
            &ed25519_spki_der(wrong_key_pair.public_key().as_ref()),
        );
        let wrong_decoding_key = DecodingKey::from_ed_pem(wrong_public_pem.as_bytes())?;

        let user_id = UserId::try_from(Uuid::new_v4())?;
        let token = create_jwt(&user_id, &encoding_key, 60 * 60 * 24)?;

        let result = validate_jwt(&token, &wrong_decoding_key);
        assert!(result.is_err());
        assert!(matches!(result, Err(AppError::InvalidSignature(_))));
        Ok(())
    }

    #[test]
    fn rejects_garbage_token() -> anyhow::Result<()> {
        let (_, public_pem) = dev_keypair_pems()?;
        let decoding_key = DecodingKey::from_ed_pem(public_pem.as_bytes())?;
        let result = validate_jwt("this.is.not.a.jwt", &decoding_key);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn rejects_expired_token() -> anyhow::Result<()> {
        let (private_pem, public_pem) = dev_keypair_pems()?;

        let user_id = UserId::try_from(Uuid::new_v4())?;
        let expired = Claims {
            sub: user_id,
            exp: 0,
            iat: 0,
            aud: JWT_AUD.to_string(),
            iss: JWT_ISS.to_string(),
            jti: Uuid::new_v4(),
        };

        let header = Header::new(Algorithm::EdDSA);
        let key = EncodingKey::from_ed_pem(private_pem.as_bytes())?;
        let token = jsonwebtoken::encode(&header, &expired, &key)?;

        let decoding_key = DecodingKey::from_ed_pem(public_pem.as_bytes())?;

        let result = validate_jwt(&token, &decoding_key);
        assert!(result.is_err());
        assert!(matches!(result, Err(AppError::TokenExpired(_))));
        Ok(())
    }
}
