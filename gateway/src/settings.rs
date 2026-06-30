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

use crate::pem::{ed25519_spki_der, pem_encode};

/// `EdDSA` JWT issuer and audience value — shared between encoding and decoding.
pub const JWT_ISS: &str = "gateway-example";

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
    /// Shared password that any `user_id` may submit to obtain a token.
    /// Replace with a real user database in production.
    pub default_admin_password: Arc<str>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("jwt_private_key_pem", &"<redacted>")
            .field("jwt_public_key_pem", &"<redacted>")
            .field("default_admin_password", &"<redacted>")
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

        Ok(Self {
            jwt_private_key_pem: Arc::from(jwt_private_key_pem.as_str()),
            jwt_public_key_pem: Arc::from(jwt_public_key_pem.as_str()),
            default_admin_password: Arc::from(default_admin_password.as_str()),
        })
    }

    /// Load settings with a freshly-generated ephemeral `EdDSA` keypair.
    ///
    /// This is intended for local development only. The generated keypair is
    /// logged at `warn!` level so operators are aware that keys are ephemeral.
    ///
    /// # Panics
    ///
    /// Panics only on programmer error (e.g. PEM encoding fails), never on
    /// runtime conditions.
    ///
    /// # Errors
    ///
    /// Returns an error if `ADMIN_PASSWORD` is not set or is empty.
    pub fn load_dev_keys() -> Result<Self, anyhow::Error> {
        use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

        let key_pair = Ed25519KeyPair::generate()
            .map_err(|_| anyhow::anyhow!("Ed25519 key generation failed"))?;

        let pkcs8_doc = key_pair
            .to_pkcs8v1()
            .map_err(|_| anyhow::anyhow!("Ed25519 PKCS#8 v1 encoding failed"))?;
        let public_key = key_pair.public_key().as_ref();

        let private_pem = pem_encode("PRIVATE KEY", pkcs8_doc.as_ref());
        let public_pem = pem_encode("PUBLIC KEY", &ed25519_spki_der(public_key));

        tracing::warn!("─── DEV KEYPAIR (ephemeral, do not use in production) ───");
        tracing::warn!("JWT_PRIVATE_KEY_PEM={private_pem}");
        tracing::warn!("JWT_PUBLIC_KEY_PEM={public_pem}");
        tracing::warn!("─── END DEV KEYPAIR ───");

        let default_admin_password = std::env::var("ADMIN_PASSWORD")
            .map_err(|_| anyhow::anyhow!("ADMIN_PASSWORD must be set"))?;

        if default_admin_password.is_empty() {
            anyhow::bail!("ADMIN_PASSWORD must not be empty");
        }

        Ok(Self {
            jwt_private_key_pem: Arc::from(private_pem.as_str()),
            jwt_public_key_pem: Arc::from(public_pem.as_str()),
            default_admin_password: Arc::from(default_admin_password.as_str()),
        })
    }
}
