//! Shared gateway settings.
//!
//! Secret/auth material still comes from deployment environment variables.
//! Non-secret runtime settings are overwritten from `rwf_config::Config` by
//! production `main`, keeping the typed config tree authoritative.

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context as _;
use jsonwebtoken::{DecodingKey, EncodingKey};
use rwf_domain::UserId;

use crate::pem::{ed25519_spki_der, pem_encode};

pub const JWT_ISS: &str = "gateway-example";
pub const JWT_AUD: &str = "gateway-example-api";
pub const DEFAULT_ADMIN_USER_ID: &str = "00000000-0000-0000-0000-000000000001";
pub const DEFAULT_ALLOWED_ORIGINS: &str =
    "http://localhost:3000,http://localhost:3001,http://localhost:3002";

#[must_use]
fn short_fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn bool_env(name: &str, default: bool) -> anyhow::Result<bool> {
    match std::env::var(name) {
        Ok(value) if value.eq_ignore_ascii_case("true") || value == "1" => Ok(true),
        Ok(value) if value.eq_ignore_ascii_case("false") || value == "0" => Ok(false),
        Ok(value) => anyhow::bail!("{name} must be true/false or 1/0, got {value:?}"),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(e) => Err(e).with_context(|| format!("failed to read {name}")),
    }
}

fn positive_i64_env(name: &str, default: i64) -> anyhow::Result<i64> {
    let value = match std::env::var(name) {
        Ok(raw) => raw
            .parse::<i64>()
            .with_context(|| format!("{name} must be an integer, got {raw:?}"))?,
        Err(std::env::VarError::NotPresent) => default,
        Err(e) => return Err(e).with_context(|| format!("failed to read {name}")),
    };
    if value <= 0 {
        anyhow::bail!("{name} must be positive, got {value}");
    }
    Ok(value)
}

fn positive_usize_env(name: &str, default: usize) -> anyhow::Result<usize> {
    let value = match std::env::var(name) {
        Ok(raw) => raw
            .parse::<usize>()
            .with_context(|| format!("{name} must be a positive integer, got {raw:?}"))?,
        Err(std::env::VarError::NotPresent) => default,
        Err(e) => return Err(e).with_context(|| format!("failed to read {name}")),
    };
    if value == 0 {
        anyhow::bail!("{name} must be greater than zero");
    }
    Ok(value)
}

#[derive(Debug, Clone)]
pub struct SessionSettings {
    pub cookie_secure: bool,
    pub cookie_name: String,
    pub csrf_cookie_name: String,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            cookie_secure: true,
            cookie_name: "rwf_session".to_string(),
            csrf_cookie_name: "rwf_csrf".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct Settings {
    pub jwt_private_key_pem: Arc<str>,
    pub jwt_public_key_pem: Arc<str>,
    pub encoding_key: Arc<EncodingKey>,
    pub decoding_key: Arc<DecodingKey>,
    pub access_token_ttl_secs: i64,
    pub admin_user_id: UserId,
    pub default_admin_password: Arc<str>,
    pub allowed_origins: Arc<str>,
    pub sse_broadcast_buffer: usize,
    pub session: SessionSettings,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("jwt_private_key_pem", &"<redacted>")
            .field("jwt_public_key_pem", &"<redacted>")
            .field("encoding_key", &"<redacted>")
            .field("decoding_key", &"<redacted>")
            .field("access_token_ttl_secs", &self.access_token_ttl_secs)
            .field("admin_user_id", &self.admin_user_id)
            .field("default_admin_password", &"<redacted>")
            .field("allowed_origins", &self.allowed_origins)
            .field("sse_broadcast_buffer", &self.sse_broadcast_buffer)
            .field("session", &self.session)
            .finish()
    }
}

impl Settings {
    /// Load secrets plus legacy environment fallbacks. Production `main`
    /// replaces non-secret values with the already-validated typed config.
    ///
    /// # Errors
    /// Returns an error for missing/invalid keys, credentials, IDs or values.
    pub fn load() -> Result<Self, anyhow::Error> {
        let jwt_private_key_pem = std::env::var("JWT_PRIVATE_KEY_PEM")
            .map_err(|_| anyhow::anyhow!("JWT_PRIVATE_KEY_PEM must be set"))?;
        let jwt_public_key_pem = std::env::var("JWT_PUBLIC_KEY_PEM")
            .map_err(|_| anyhow::anyhow!("JWT_PUBLIC_KEY_PEM must be set"))?;
        let default_admin_password = std::env::var("ADMIN_PASSWORD")
            .map_err(|_| anyhow::anyhow!("ADMIN_PASSWORD must be set"))?;

        if jwt_private_key_pem.is_empty() || !jwt_private_key_pem.starts_with("-----BEGIN ") {
            anyhow::bail!("JWT_PRIVATE_KEY_PEM must contain PEM-encoded key material");
        }
        if jwt_public_key_pem.is_empty() || !jwt_public_key_pem.starts_with("-----BEGIN ") {
            anyhow::bail!("JWT_PUBLIC_KEY_PEM must contain PEM-encoded key material");
        }
        if default_admin_password.is_empty() {
            anyhow::bail!("ADMIN_PASSWORD must not be empty");
        }

        let encoding_key = Arc::new(
            EncodingKey::from_ed_pem(jwt_private_key_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("failed to parse EdDSA private key PEM: {e}"))?,
        );
        let decoding_key = Arc::new(
            DecodingKey::from_ed_pem(jwt_public_key_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("failed to parse EdDSA public key PEM: {e}"))?,
        );

        let admin_user_id_raw =
            std::env::var("ADMIN_USER_ID").unwrap_or_else(|_| DEFAULT_ADMIN_USER_ID.to_string());
        let admin_user_id = UserId::from_str(&admin_user_id_raw)
            .with_context(|| format!("invalid ADMIN_USER_ID {admin_user_id_raw:?}"))?;

        let session = SessionSettings {
            cookie_secure: bool_env("SESSION_COOKIE_SECURE", true)?,
            cookie_name: std::env::var("SESSION_COOKIE_NAME")
                .unwrap_or_else(|_| "rwf_session".to_string()),
            csrf_cookie_name: std::env::var("CSRF_COOKIE_NAME")
                .unwrap_or_else(|_| "rwf_csrf".to_string()),
        };

        Ok(Self {
            jwt_private_key_pem: Arc::from(jwt_private_key_pem.as_str()),
            jwt_public_key_pem: Arc::from(jwt_public_key_pem.as_str()),
            encoding_key,
            decoding_key,
            access_token_ttl_secs: positive_i64_env("ACCESS_TOKEN_TTL_SECS", 15 * 60)?,
            admin_user_id,
            default_admin_password: Arc::from(default_admin_password.as_str()),
            allowed_origins: Arc::from(
                std::env::var("ALLOWED_ORIGINS")
                    .unwrap_or_else(|_| DEFAULT_ALLOWED_ORIGINS.to_string()),
            ),
            sse_broadcast_buffer: positive_usize_env("SSE_BROADCAST_BUFFER", 256)?,
            session,
        })
    }

    /// Development settings with an ephemeral keypair and HTTP-friendly
    /// session cookie. Intended only for local/test use.
    ///
    /// # Errors
    /// Returns an error when key generation/encoding fails or password is empty.
    pub fn load_dev_keys(admin_password: &str) -> Result<Self, anyhow::Error> {
        use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

        if admin_password.is_empty() {
            anyhow::bail!("admin_password must not be empty");
        }

        let key_pair = Ed25519KeyPair::generate()
            .map_err(|_| anyhow::anyhow!("Ed25519 key generation failed"))?;
        let pkcs8_doc = key_pair
            .to_pkcs8v1()
            .map_err(|_| anyhow::anyhow!("Ed25519 PKCS#8 v1 encoding failed"))?;
        let public_key = key_pair.public_key().as_ref();

        let private_pem = pem_encode("PRIVATE KEY", pkcs8_doc.as_ref());
        let public_pem = pem_encode("PUBLIC KEY", &ed25519_spki_der(public_key));
        tracing::warn!(
            priv_fingerprint = %short_fingerprint(private_pem.as_bytes()),
            pub_fingerprint = %short_fingerprint(public_pem.as_bytes()),
            "DEV KEYPAIR active (ephemeral; do not use in production)"
        );

        let encoding_key = Arc::new(
            EncodingKey::from_ed_pem(private_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("failed to parse dev EdDSA private key PEM: {e}"))?,
        );
        let decoding_key = Arc::new(
            DecodingKey::from_ed_pem(public_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("failed to parse dev EdDSA public key PEM: {e}"))?,
        );
        let admin_user_id = UserId::from_str(DEFAULT_ADMIN_USER_ID)
            .context("built-in development admin user id is invalid")?;

        Ok(Self {
            jwt_private_key_pem: Arc::from(private_pem.as_str()),
            jwt_public_key_pem: Arc::from(public_pem.as_str()),
            encoding_key,
            decoding_key,
            access_token_ttl_secs: 15 * 60,
            admin_user_id,
            default_admin_password: Arc::from(admin_password),
            allowed_origins: Arc::from(DEFAULT_ALLOWED_ORIGINS),
            sse_broadcast_buffer: 256,
            session: SessionSettings {
                cookie_secure: false,
                ..SessionSettings::default()
            },
        })
    }

    /// Load development settings using `ADMIN_PASSWORD` from the environment.
    ///
    /// # Errors
    /// Returns an error if `ADMIN_PASSWORD` is missing or development key
    /// generation/encoding fails.
    pub fn load_dev_keys_from_env() -> Result<Self, anyhow::Error> {
        let admin_password = std::env::var("ADMIN_PASSWORD")
            .map_err(|_| anyhow::anyhow!("ADMIN_PASSWORD must be set when --dev-keys is used"))?;
        Self::load_dev_keys(&admin_password)
    }
}
