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
//! should cache the parsed [`SigningKey`] / [`VerifyingKey`] objects (see
//! [`super::store`]).

use crate::settings::JWT_ISS;
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use super::error::AppError;

/// JWT claims payload.
///
/// Standard fields: `sub` (subject), `exp` (expiration), `iat` (issued-at),
/// `aud` (audience), `iss` (issuer).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
#[must_use]
pub struct Claims {
    pub sub: String,
    pub exp: u64,
    pub iat: u64,
    pub aud: String,
    pub iss: String,
}

/// Convert a [`chrono::DateTime<Utc>`] to a `u64` Unix timestamp, returning an
/// [`AppError::Internal`] on overflow (e.g. dates before epoch).
fn unix_seconds(t: chrono::DateTime<Utc>) -> Result<u64, AppError> {
    u64::try_from(t.timestamp()).map_err(|e| AppError::internal("timestamp overflow", e))
}

/// Create a signed `EdDSA` JWT for the given `user_id`.
///
/// The token expires 24 hours from creation. The PEM string is parsed on every
/// call; for hot paths consider caching the [`EncodingKey`].
///
/// # Errors
///
/// Returns [`AppError::Internal`] if the PEM key is malformed or encoding
/// fails.
pub fn create_jwt(user_id: &str, jwt_private_key_pem: &str) -> Result<String, AppError> {
    let now = Utc::now();
    let exp = unix_seconds(now + chrono::Duration::hours(24))?;

    let claims = Claims {
        sub: user_id.to_string(),
        iat: unix_seconds(now)?,
        exp,
        aud: JWT_ISS.to_string(),
        iss: JWT_ISS.to_string(),
    };

    let header = Header::new(Algorithm::EdDSA);
    let key = EncodingKey::from_ed_pem(jwt_private_key_pem.as_bytes())
        .map_err(|e| AppError::internal("failed to parse `EdDSA` private key PEM", e))?;

    encode(&header, &claims, &key).map_err(|e| AppError::internal("JWT encoding", e))
}

/// Validate an `EdDSA` JWT and return its [`Claims`].
///
/// # Errors
///
/// Returns [`AppError::TokenExpired`] if the token has expired,
/// [`AppError::InvalidSignature`] if the signature is invalid, or
/// [`AppError::Jwt`] for other decoding / PEM-parsing errors.
pub fn validate_jwt(token: &str, jwt_public_key_pem: &str) -> Result<Claims, AppError> {
    use jsonwebtoken::errors::ErrorKind;

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[JWT_ISS]);
    validation.set_audience(&[JWT_ISS]);

    let key = DecodingKey::from_ed_pem(jwt_public_key_pem.as_bytes())
        .map_err(|e| AppError::internal("failed to parse `EdDSA` public key PEM", e))?;

    decode::<Claims>(token, &key, &validation)
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

        // Sign
        let token = create_jwt("test-user", &private_pem)?;

        // Verify
        let claims = validate_jwt(&token, &public_pem)?;
        assert_eq!(claims.sub, "test-user");
        assert_eq!(claims.iss, JWT_ISS);
        assert_eq!(claims.aud, JWT_ISS);
        assert!(claims.exp > claims.iat);
        Ok(())
    }

    #[test]
    fn rejects_wrong_key() -> anyhow::Result<()> {
        let (private_pem, _) = dev_keypair_pems()?;
        // Different seed → different key pair
        let wrong_seed = [2u8; 32];
        let wrong_key_pair = Ed25519KeyPair::from_seed_unchecked(&wrong_seed)?;
        let wrong_public_pem = pem_encode(
            "PUBLIC KEY",
            &ed25519_spki_der(wrong_key_pair.public_key().as_ref()),
        );

        let token = create_jwt("test-user", &private_pem)?;

        let result = validate_jwt(&token, &wrong_public_pem);
        assert!(result.is_err());
        assert!(matches!(result, Err(AppError::InvalidSignature(_))));
        Ok(())
    }

    #[test]
    fn rejects_garbage_token() -> anyhow::Result<()> {
        let (_, public_pem) = dev_keypair_pems()?;
        let result = validate_jwt("this.is.not.a.jwt", &public_pem);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn rejects_expired_token() -> anyhow::Result<()> {
        let (private_pem, public_pem) = dev_keypair_pems()?;

        // Manually craft a token with exp = 0 (i.e., well in the past).
        let expired = Claims {
            sub: "user-1".to_string(),
            exp: 0,
            iat: 0,
            aud: JWT_ISS.to_string(),
            iss: JWT_ISS.to_string(),
        };

        let header = Header::new(Algorithm::EdDSA);
        let key = EncodingKey::from_ed_pem(private_pem.as_bytes())?;
        let token = jsonwebtoken::encode(&header, &expired, &key)?;

        // Validate — should fail with TokenExpired
        let result = validate_jwt(&token, &public_pem);
        assert!(result.is_err());
        assert!(matches!(result, Err(AppError::TokenExpired(_))));
        Ok(())
    }
}
