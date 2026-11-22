//! Shared gateway settings.
//!
//! Credentials and typed runtime settings are supplied by the executable or caller.

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context as _;
use jsonwebtoken::{DecodingKey, EncodingKey};
use rwf_domain::UserId;

use crate::pem::{ed25519_spki_der, pem_encode};

pub const JWT_ISS: &str = "gateway-example";
pub const JWT_AUD: &str = "gateway-example-api";

fn token_ttls(config: &rwf_config::GatewayConfig) -> Result<(i64, i64), anyhow::Error> {
    let access = i64::try_from(config.access_token_ttl_secs)
        .context("gateway.access_token_ttl_secs exceeds i64::MAX")?;
    let refresh = i64::try_from(config.refresh_token_ttl_secs)
        .context("gateway.refresh_token_ttl_secs exceeds i64::MAX")?;
    Ok((access, refresh))
}

#[must_use]
fn short_fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Clone)]
pub struct Settings {
    pub jwt_private_key_pem: Arc<str>,
    pub jwt_public_key_pem: Arc<str>,
    pub encoding_key: Arc<EncodingKey>,
    pub decoding_key: Arc<DecodingKey>,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_secs: i64,
    pub admin_user_id: UserId,
    pub default_admin_password: Arc<str>,
    pub allowed_origins: Arc<str>,
    pub sse_broadcast_buffer: usize,
    pub session: rwf_config::SessionConfig,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("jwt_private_key_pem", &"<redacted>")
            .field("jwt_public_key_pem", &"<redacted>")
            .field("encoding_key", &"<redacted>")
            .field("decoding_key", &"<redacted>")
            .field("access_token_ttl_secs", &self.access_token_ttl_secs)
            .field("refresh_token_ttl_secs", &self.refresh_token_ttl_secs)
            .field("admin_user_id", &self.admin_user_id)
            .field("default_admin_password", &"<redacted>")
            .field("allowed_origins", &self.allowed_origins)
            .field("sse_broadcast_buffer", &self.sse_broadcast_buffer)
            .field("session", &self.session)
            .finish()
    }
}

impl Settings {
    /// Construct settings from supplied key material and typed runtime configuration.
    ///
    /// # Errors
    /// Returns an error for invalid key material, credentials, user identity, or TTLs.
    pub fn from_credentials(
        jwt_private_key_pem: &str,
        jwt_public_key_pem: &str,
        default_admin_password: &str,
        config: &rwf_config::GatewayConfig,
    ) -> Result<Self, anyhow::Error> {
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

        let admin_user_id =
            UserId::from_str(&config.admin_user_id).context("invalid gateway.admin_user_id")?;
        let (access_token_ttl_secs, refresh_token_ttl_secs) = token_ttls(config)?;

        Ok(Self {
            jwt_private_key_pem: Arc::from(jwt_private_key_pem),
            jwt_public_key_pem: Arc::from(jwt_public_key_pem),
            encoding_key,
            decoding_key,
            access_token_ttl_secs,
            refresh_token_ttl_secs,
            admin_user_id,
            default_admin_password: Arc::from(default_admin_password),
            allowed_origins: Arc::from(config.cors.allowed_origins.as_str()),
            sse_broadcast_buffer: config.sse_broadcast_buffer,
            session: config.session,
        })
    }

    /// Development settings with an ephemeral keypair. Intended only for local/test use.
    /// Session policy still comes from the explicit configuration.
    ///
    /// # Errors
    /// Returns an error when key generation/encoding fails or credentials/configuration are invalid.
    pub fn generate_dev_keys(
        admin_password: &str,
        config: &rwf_config::GatewayConfig,
    ) -> Result<Self, anyhow::Error> {
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

        Self::from_credentials(&private_pem, &public_pem, admin_password, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_key_modes_share_explicit_runtime_settings() -> anyhow::Result<()> {
        let config = rwf_config::GatewayConfig {
            admin_user_id: "00000000-0000-0000-0000-000000000002".to_owned(),
            session: rwf_config::SessionConfig {
                cookie_secure: false,
            },
            cors: rwf_config::CorsConfig {
                allowed_origins: "https://example.test".to_owned(),
            },
            sse_broadcast_buffer: 42,
            ..rwf_config::GatewayConfig::default()
        };
        let dev = Settings::generate_dev_keys("test-password", &config)?;
        let supplied = Settings::from_credentials(
            &dev.jwt_private_key_pem,
            &dev.jwt_public_key_pem,
            "another-password",
            &config,
        )?;
        for settings in [dev, supplied] {
            assert_eq!(settings.admin_user_id.to_string(), config.admin_user_id);
            assert!(!settings.session.cookie_secure);
            assert_eq!(settings.sse_broadcast_buffer, 42);
            assert_eq!(&*settings.allowed_origins, "https://example.test");
        }
        Ok(())
    }

    #[test]
    fn runtime_config_applies_both_token_ttls() -> anyhow::Result<()> {
        let config = rwf_config::GatewayConfig {
            access_token_ttl_secs: 120,
            refresh_token_ttl_secs: 43_200,
            ..rwf_config::GatewayConfig::default()
        };
        let settings = Settings::generate_dev_keys("test-password", &config)?;

        assert_eq!(settings.access_token_ttl_secs, 120);
        assert_eq!(settings.refresh_token_ttl_secs, 43_200);
        Ok(())
    }
}
