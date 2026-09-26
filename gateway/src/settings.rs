//! Shared gateway settings.
//!
//! Secret/auth material comes from deployment environment variables. Non-secret
//! runtime settings are applied explicitly from `rwf_config::GatewayConfig`.

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context as _;
use jsonwebtoken::{DecodingKey, EncodingKey};
use rwf_domain::UserId;

use crate::pem::{ed25519_spki_der, pem_encode};

pub const JWT_ISS: &str = "gateway-example";
pub const JWT_AUD: &str = "gateway-example-api";
pub const DEFAULT_ADMIN_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

#[must_use]
fn short_fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
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
    /// Apply the canonical typed runtime configuration.
    ///
    /// # Errors
    /// Returns an error if a validated integer cannot be represented by the
    /// gateway's signed duration fields.
    pub fn apply_runtime_config(
        mut self,
        config: &rwf_config::GatewayConfig,
    ) -> Result<Self, anyhow::Error> {
        self.access_token_ttl_secs = i64::try_from(config.access_token_ttl_secs)
            .context("gateway.access_token_ttl_secs exceeds i64::MAX")?;
        self.allowed_origins = Arc::from(config.cors.allowed_origins.as_str());
        self.sse_broadcast_buffer = config.sse_broadcast_buffer;
        self.session.cookie_secure = config.session.cookie_secure;
        Ok(self)
    }

    /// Load secret/auth deployment settings.
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
            cookie_name: std::env::var("SESSION_COOKIE_NAME")
                .unwrap_or_else(|_| "rwf_session".to_string()),
            csrf_cookie_name: std::env::var("CSRF_COOKIE_NAME")
                .unwrap_or_else(|_| "rwf_csrf".to_string()),
            ..SessionSettings::default()
        };

        Ok(Self {
            jwt_private_key_pem: Arc::from(jwt_private_key_pem.as_str()),
            jwt_public_key_pem: Arc::from(jwt_public_key_pem.as_str()),
            encoding_key,
            decoding_key,
            access_token_ttl_secs: 15 * 60,
            admin_user_id,
            default_admin_password: Arc::from(default_admin_password.as_str()),
            allowed_origins: Arc::from(
                "http://localhost:3000,http://localhost:3001,http://localhost:3002",
            ),
            sse_broadcast_buffer: 256,
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
            allowed_origins: Arc::from(
                "http://localhost:3000,http://localhost:3001,http://localhost:3002",
            ),
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
