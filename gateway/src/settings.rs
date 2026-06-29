//! Shared configuration for the gateway example.
//!
//! Settings are loaded from environment variables at startup.
//! Secret fields are redacted in [`Debug`] output.

/// Shared configuration loaded from environment variables.
///
/// All secret fields are redacted in [`Debug`] output.
#[derive(Clone)]
pub struct Settings {
    pub jwt_secret: String,
    pub default_admin_password: String,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("jwt_secret", &"<redacted>")
            .field("default_admin_password", &"<redacted>")
            .finish()
    }
}

impl Settings {
    /// Load settings from environment variables.
    ///
    /// Reads `JWT_SECRET` and `ADMIN_PASSWORD` from the environment.
    /// Fails with a descriptive error if either is missing or invalid.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * `JWT_SECRET` is not set or is empty.
    /// * `JWT_SECRET` is shorter than 32 bytes.
    /// * `JWT_SECRET` is the placeholder value `"changeme-in-production"`.
    /// * `ADMIN_PASSWORD` is not set or is empty.
    pub fn load() -> Result<Self, anyhow::Error> {
        let jwt_secret =
            std::env::var("JWT_SECRET").map_err(|_| anyhow::anyhow!("JWT_SECRET must be set"))?;

        let default_admin_password = std::env::var("ADMIN_PASSWORD")
            .map_err(|_| anyhow::anyhow!("ADMIN_PASSWORD must be set"))?;

        if jwt_secret.is_empty() {
            anyhow::bail!("JWT_SECRET must not be empty");
        }
        if jwt_secret.len() < 32 {
            anyhow::bail!(
                "JWT_SECRET must be at least 32 bytes long (got {})",
                jwt_secret.len()
            );
        }
        if jwt_secret == "changeme-in-production" {
            anyhow::bail!("JWT_SECRET must not be the placeholder value");
        }
        if default_admin_password.is_empty() {
            anyhow::bail!("ADMIN_PASSWORD must not be empty");
        }

        Ok(Self {
            jwt_secret,
            default_admin_password,
        })
    }
}
