//! Shared configuration for the gateway example.
//!
//! Settings are loaded from environment variables at startup, or generated for
//! development via the `--dev-keys` CLI flag.
//!
//! # `EdDSA` Keypair
//!
//! The gateway uses Ed25519 (`EdDSA`) for JWT signing and verification.  The
//! private key is stored as a PKCS#8 PEM string (`JWT_PRIVATE_KEY_PEM`), and
//! the public key as an SPKI PEM string (`JWT_PUBLIC_KEY_PEM`).
//!
//! In development, pass `--dev-keys` to the binary to generate an ephemeral
//! keypair at startup (logged at `warn!` level so operators see it).
//!
//! # Security
//!
//! Secret fields are redacted in [`Debug`] output and stored as [`Arc<str>`]
//! (cheap to clone, share-on-write, no per-clone heap allocation).

use std::sync::Arc;

use jsonwebtoken::{DecodingKey, EncodingKey};

use crate::pem::{ed25519_spki_der, pem_encode};

/// `EdDSA` JWT issuer value — published in the `iss` claim.
///
/// The issuer identifies the party that minted the token (this gateway).
pub const JWT_ISS: &str = "gateway-example";

/// `EdDSA` JWT audience value — published in the `aud` claim.
///
/// The audience identifies the intended consumer of the token. By RFC 7519
/// §4.1.3 the audience should be distinct from the issuer unless the producer
/// and consumer are the same entity; we follow the standard practice here
/// (`gateway-example` for the issuer, `gateway-example-api` for the audience)
/// so future token-replay scenarios across cooperating services can
/// differentiate their accept policies without changing JWT minting.
pub const JWT_AUD: &str = "gateway-example-api";

/// FNV-1a 64-bit digest of `bytes`, formatted as 16 hex chars.
///
/// Used to produce a short, stable fingerprint of PEM key material without
/// logging the full key (avoiding new dependencies like `sha2`).
#[must_use]
fn short_fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64-bit offset basis
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3); // FNV prime
    }
    format!("{h:016x}")
}

/// Session-cookie and CSRF configuration.
///
/// Controls the session cookie attributes and the CSRF token cookie name.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct SessionSettings {
    /// Whether the session cookie requires HTTPS (`Secure` flag).
    pub cookie_secure: bool,
    /// Name of the session cookie (default `"rwf_session"`).
    pub cookie_name: String,
    /// Name of the CSRF cookie (default `"rwf_csrf"`).
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

/// Shared configuration loaded from environment variables.
///
/// Secrets are stored as [`Arc<str>`] so cloning [`Settings`] (which
/// [`axum::extract::State`] does on every request) is a refcount bump rather
/// than a deep `String` clone. All secret fields are redacted in
/// [`Debug`] output.
#[derive(Clone)]
pub struct Settings {
    /// Ed25519 private key in PKCS#8 PEM format.
    pub jwt_private_key_pem: Arc<str>,
    /// Ed25519 public key in SPKI PEM format.
    pub jwt_public_key_pem: Arc<str>,
    /// Pre-parsed `EdDSA` encoding key (derived from `jwt_private_key_pem`).
    pub encoding_key: Arc<EncodingKey>,
    /// Pre-parsed `EdDSA` decoding key (derived from `jwt_public_key_pem`).
    pub decoding_key: Arc<DecodingKey>,
    /// Access-token lifetime in seconds. Short-lived by design; DB-backed
    /// refresh tokens carry the long-lived credential. Default: 15 min.
    pub access_token_ttl_secs: i64,
    /// Shared password that any `user_id` may submit to obtain a token.
    /// Replace with a real user database in production.
    pub default_admin_password: Arc<str>,
    /// Session-cookie and CSRF settings.
    pub session: SessionSettings,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("jwt_private_key_pem", &"<redacted>")
            .field("jwt_public_key_pem", &"<redacted>")
            .field("encoding_key", &"<redacted>")
            .field("decoding_key", &"<redacted>")
            .field("default_admin_password", &"<redacted>")
            .field("session", &self.session)
            .finish()
    }
}

impl Settings {
    /// Load settings from environment variables.
    ///
    /// Reads `JWT_PRIVATE_KEY_PEM`, `JWT_PUBLIC_KEY_PEM`, and `ADMIN_PASSWORD`
    /// from the environment. Fails with a descriptive error if any is missing
    /// or invalid.
    ///
    /// # Errors
    ///
    /// Returns an error if any required env var is unset, empty, or fails
    /// validation.
    pub fn load() -> Result<Self, anyhow::Error> {
        let jwt_private_key_pem = std::env::var("JWT_PRIVATE_KEY_PEM")
            .map_err(|_| anyhow::anyhow!("JWT_PRIVATE_KEY_PEM must be set"))?;
        let jwt_public_key_pem = std::env::var("JWT_PUBLIC_KEY_PEM")
            .map_err(|_| anyhow::anyhow!("JWT_PUBLIC_KEY_PEM must be set"))?;
        let default_admin_password = std::env::var("ADMIN_PASSWORD")
            .map_err(|_| anyhow::anyhow!("ADMIN_PASSWORD must be set"))?;

        if jwt_private_key_pem.is_empty() {
            anyhow::bail!("JWT_PRIVATE_KEY_PEM must not be empty");
        }
        if !jwt_private_key_pem.starts_with("-----BEGIN ") {
            anyhow::bail!("JWT_PRIVATE_KEY_PEM does not look like a valid PEM-encoded key");
        }
        if jwt_public_key_pem.is_empty() {
            anyhow::bail!("JWT_PUBLIC_KEY_PEM must not be empty");
        }
        if !jwt_public_key_pem.starts_with("-----BEGIN ") {
            anyhow::bail!("JWT_PUBLIC_KEY_PEM does not look like a valid PEM-encoded key");
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

        let session = SessionSettings {
            cookie_secure: std::env::var("SESSION_COOKIE_SECURE")
                .ok()
                .is_none_or(|v| v == "true"),
            cookie_name: std::env::var("SESSION_COOKIE_NAME")
                .ok()
                .unwrap_or_else(|| "rwf_session".to_string()),
            csrf_cookie_name: std::env::var("CSRF_COOKIE_NAME")
                .ok()
                .unwrap_or_else(|| "rwf_csrf".to_string()),
        };

        let access_token_ttl_secs: i64 = std::env::var("ACCESS_TOKEN_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15 * 60); // 15 minutes — short by design

        Ok(Self {
            jwt_private_key_pem: Arc::from(jwt_private_key_pem.as_str()),
            jwt_public_key_pem: Arc::from(jwt_public_key_pem.as_str()),
            encoding_key,
            decoding_key,
            access_token_ttl_secs,
            default_admin_password: Arc::from(default_admin_password.as_str()),
            session,
        })
    }

    /// Load settings with a freshly-generated ephemeral `EdDSA` keypair.
    ///
    /// This is intended for local development only. The generated keypair is
    /// logged at `warn!` level so operators are aware that keys are ephemeral.
    ///
    /// The `admin_password` is taken as an argument so callers (including tests)
    /// can construct `Settings` without mutating process-wide environment
    /// variables.
    ///
    /// # Errors
    ///
    /// Returns an error if the ephemeral keypair cannot be generated or encoded,
    /// or if `admin_password` is empty.
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

        let priv_fp = short_fingerprint(private_pem.as_bytes());
        let pub_fp = short_fingerprint(public_pem.as_bytes());
        tracing::warn!(
            priv_fingerprint = %priv_fp,
            pub_fingerprint = %pub_fp,
            "─── DEV KEYPAIR active (ephemeral, do not use in production). \
             See settings.rs::Settings::load_dev_keys. ───"
        );

        let encoding_key = Arc::new(
            EncodingKey::from_ed_pem(private_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("failed to parse dev EdDSA private key PEM: {e}"))?,
        );
        let decoding_key = Arc::new(
            DecodingKey::from_ed_pem(public_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("failed to parse dev EdDSA public key PEM: {e}"))?,
        );

        Ok(Self {
            jwt_private_key_pem: Arc::from(private_pem.as_str()),
            jwt_public_key_pem: Arc::from(public_pem.as_str()),
            encoding_key,
            decoding_key,
            access_token_ttl_secs: 15 * 60, // 15 minutes — short by design
            default_admin_password: Arc::from(admin_password),
            session: SessionSettings::default(),
        })
    }

    /// Load dev-key settings using `ADMIN_PASSWORD` from the environment.
    ///
    /// Thin wrapper around [`Self::load_dev_keys`] for the production `--dev-keys`
    /// CLI path, where `ADMIN_PASSWORD` is conventionally provided via the
    /// process environment.
    ///
    /// # Errors
    ///
    /// Returns an error if `ADMIN_PASSWORD` is not set or is empty, or if the
    /// underlying [`Self::load_dev_keys`] call fails.
    pub fn load_dev_keys_from_env() -> Result<Self, anyhow::Error> {
        let admin_password = std::env::var("ADMIN_PASSWORD")
            .map_err(|_| anyhow::anyhow!("ADMIN_PASSWORD must be set when --dev-keys is used"))?;
        Self::load_dev_keys(&admin_password)
    }
}
