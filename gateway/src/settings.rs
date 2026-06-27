/// Hardcoded shared settings for the gateway example.
///
/// In production these would come from a config file, environment variables,
/// or a secret store.  Keeping them here keeps the example self-contained.
#[derive(Debug, Clone)]
pub struct Settings {
    pub jwt_secret: String,
    pub default_admin_password: String,
}

impl Settings {
    #[must_use]
    pub fn load() -> Self {
        Self {
            jwt_secret: "changeme-in-production".into(),
            default_admin_password: "admin".into(),
        }
    }
}
