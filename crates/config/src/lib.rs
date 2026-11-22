//! Typed workspace configuration from explicit TOML or checked defaults.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

const MAX_I64_AS_U64: u64 = 9_223_372_036_854_775_807;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub live_search: LiveSearchConfig,
    pub i18n_demo: I18nConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GatewayConfig {
    pub port: u16,
    pub proxy_upstream_url: String,
    pub cors: CorsConfig,
    pub session: SessionConfig,
    pub admin_user_id: String,
    pub allow_dev_keys: bool,
    pub sse_broadcast_buffer: usize,
    pub refresh_token_ttl_secs: u64,
    pub access_token_ttl_secs: u64,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 3001,
            proxy_upstream_url: "https://ipapi.co".to_string(),
            cors: CorsConfig::default(),
            session: SessionConfig::default(),
            admin_user_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            allow_dev_keys: false,
            sse_broadcast_buffer: 256,
            refresh_token_ttl_secs: 60 * 60 * 24 * 30,
            access_token_ttl_secs: 15 * 60,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CorsConfig {
    pub allowed_origins: String,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: "http://localhost:3000,http://localhost:3001,http://localhost:3002"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    pub cookie_secure: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            cookie_secure: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LiveSearchConfig {
    pub port: u16,
    pub pool_max_connections: u32,
    pub pool_min_connections: u32,
    pub pool_acquire_timeout_secs: u64,
    pub pool_idle_timeout_secs: u64,
    pub pool_max_lifetime_secs: u64,
    pub sse_broadcast_buffer: usize,
}

impl Default for LiveSearchConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            pool_max_connections: 20,
            pool_min_connections: 2,
            pool_acquire_timeout_secs: 5,
            pool_idle_timeout_secs: 600,
            pool_max_lifetime_secs: 1800,
            sse_broadcast_buffer: 256,
        }
    }
}

impl LiveSearchConfig {
    #[must_use = "the returned summary describes the database connection budget"]
    pub fn connection_budget_summary(&self) -> String {
        format!(
            "pool max_connections={}, min_connections={}, acquire_timeout={}s, idle_timeout={}s, max_lifetime={}s",
            self.pool_max_connections,
            self.pool_min_connections,
            self.pool_acquire_timeout_secs,
            self.pool_idle_timeout_secs,
            self.pool_max_lifetime_secs,
        )
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct I18nConfig {
    pub port: u16,
}

impl Default for I18nConfig {
    fn default() -> Self {
        Self { port: 3002 }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read configuration {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid configuration: {0}")]
    Invalid(String),
    #[error("invalid startup argument: {0}")]
    Argument(String),
}

/// Selects the application-specific startup argument grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Server {
    Gateway,
    LiveSearch,
    I18nDemo,
}

/// Explicit executable arguments; independent of the process environment.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StartupOptions {
    pub config_path: Option<PathBuf>,
    pub dev_keys: bool,
}

impl StartupOptions {
    /// Parse arguments after the executable name.
    ///
    /// # Errors
    /// Rejects unknown, duplicate, or missing arguments; development keys
    /// are a gateway-only option.
    pub fn parse(
        server: Server,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, ConfigError> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--config") if options.config_path.is_none() => {
                    let path = args
                        .next()
                        .filter(|value| {
                            !value.is_empty() && !value.as_encoded_bytes().starts_with(b"--")
                        })
                        .ok_or_else(|| {
                            ConfigError::Argument("--config requires a path".to_owned())
                        })?;
                    options.config_path = Some(path.into());
                }
                Some("--dev-keys") if server == Server::Gateway && !options.dev_keys => {
                    options.dev_keys = true;
                }
                _ => {
                    return Err(ConfigError::Argument(format!(
                        "unexpected argument {}",
                        arg.to_string_lossy()
                    )));
                }
            }
        }
        Ok(options)
    }
}

impl Config {
    /// Load typed defaults or a specific TOML file, with no ambient overrides.
    ///
    /// # Errors
    /// Returns a configuration error for unreadable/invalid files or invalid
    /// cross-field invariants.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        if let Some(path) = path {
            let input = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            Self::from_toml(&input)
        } else {
            let config = Self::default();
            config.validate()?;
            Ok(config)
        }
    }

    /// Parse an explicit TOML document, using the same defaults as startup.
    ///
    /// # Errors
    /// Returns a configuration error for invalid syntax, unknown fields, or
    /// invalid cross-field invariants.
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: &str| Err(ConfigError::Invalid(message.to_string()));

        if self.gateway.proxy_upstream_url.trim().is_empty() {
            return invalid("gateway.proxy_upstream_url must not be empty");
        }
        if self.gateway.sse_broadcast_buffer == 0 {
            return invalid("gateway.sse_broadcast_buffer must be greater than zero");
        }
        if self.gateway.refresh_token_ttl_secs == 0
            || self.gateway.refresh_token_ttl_secs > MAX_I64_AS_U64
        {
            return invalid("gateway.refresh_token_ttl_secs must fit in positive i64");
        }
        if self.gateway.access_token_ttl_secs == 0
            || self.gateway.access_token_ttl_secs > MAX_I64_AS_U64
        {
            return invalid("gateway.access_token_ttl_secs must fit in positive i64");
        }

        if self.live_search.pool_max_connections == 0 {
            return invalid("live_search.pool_max_connections must be greater than zero");
        }
        if self.live_search.pool_min_connections > self.live_search.pool_max_connections {
            return invalid(
                "live_search.pool_min_connections must not exceed pool_max_connections",
            );
        }
        if self.live_search.pool_acquire_timeout_secs == 0 {
            return invalid("live_search.pool_acquire_timeout_secs must be greater than zero");
        }
        if self.live_search.pool_idle_timeout_secs == 0 {
            return invalid("live_search.pool_idle_timeout_secs must be greater than zero");
        }
        if self.live_search.pool_max_lifetime_secs == 0 {
            return invalid("live_search.pool_max_lifetime_secs must be greater than zero");
        }
        if self.live_search.sse_broadcast_buffer == 0 {
            return invalid("live_search.sse_broadcast_buffer must be greater than zero");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() -> Result<(), ConfigError> {
        let cfg = Config::load(None)?;
        assert_eq!(cfg.gateway.port, 3001);
        assert_eq!(cfg.live_search.port, 3000);
        assert_eq!(cfg.i18n_demo.port, 3002);
        assert_eq!(cfg.live_search.pool_max_connections, 20);
        assert_eq!(cfg.live_search.pool_min_connections, 2);
        assert_eq!(cfg.live_search.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.refresh_token_ttl_secs, 60 * 60 * 24 * 30);
        assert_eq!(cfg.gateway.access_token_ttl_secs, 15 * 60);
        assert!(cfg.gateway.session.cookie_secure);
        assert!(!cfg.gateway.allow_dev_keys);
        Ok(())
    }

    #[test]
    fn explicit_missing_path_errors() {
        let result = Config::load(Some(Path::new("/this/path/does/not/exist.toml")));
        assert!(matches!(result, Err(ConfigError::Read { .. })));
    }

    #[test]
    fn partial_nested_config_preserves_typed_defaults() -> Result<(), ConfigError> {
        let cfg = Config::from_toml(
            "[gateway.session]\ncookie_secure = false\n[live_search]\npool_max_connections = 50",
        )?;
        assert!(!cfg.gateway.session.cookie_secure);
        assert_eq!(cfg.gateway.port, 3001);
        assert_eq!(cfg.gateway.sse_broadcast_buffer, 256);
        assert_eq!(cfg.live_search.pool_max_connections, 50);
        assert_eq!(cfg.live_search.pool_min_connections, 2);
        assert_eq!(cfg.i18n_demo.port, 3002);
        Ok(())
    }

    #[test]
    fn checked_in_config_examples_have_expected_policy() -> Result<(), ConfigError> {
        let production = Config::from_toml(include_str!("../../../config.toml"))?;
        let development = Config::from_toml(include_str!("../../../config.dev.toml"))?;
        assert!(!production.gateway.allow_dev_keys);
        assert!(production.gateway.session.cookie_secure);
        assert!(development.gateway.allow_dev_keys);
        assert!(!development.gateway.session.cookie_secure);
        Ok(())
    }

    #[test]
    fn invalid_zero_sse_buffer_is_rejected() {
        assert!(matches!(
            Config::from_toml("[live_search]\nsse_broadcast_buffer = 0"),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn unknown_fields_and_malformed_booleans_are_rejected() {
        for input in [
            "[gateway.session]\ncookie_secure = 'false'",
            "[gateway]\nports = 4000",
            "[unknown]\nport = 4000",
            "[live_search]\ndatabase_url = 'removed'",
            "[i18n_demo]\nport = 65536",
        ] {
            assert!(
                matches!(Config::from_toml(input), Err(ConfigError::Parse(_))),
                "{input}"
            );
        }
    }

    #[test]
    fn startup_parser_is_explicit_and_rejects_invalid_modes() -> Result<(), ConfigError> {
        let options = StartupOptions::parse(
            Server::Gateway,
            ["--dev-keys", "--config", "demo.toml"].map(Into::into),
        )?;
        assert_eq!(options.config_path, Some("demo.toml".into()));
        assert!(options.dev_keys);
        assert_eq!(
            StartupOptions::parse(Server::LiveSearch, [])?,
            StartupOptions::default()
        );
        for (server, args) in [
            (Server::LiveSearch, &["--dev-keys"][..]),
            (Server::I18nDemo, &["--dev-keys"][..]),
            (Server::Gateway, &["--dev-keys", "--dev-keys"][..]),
            (Server::Gateway, &["--config"][..]),
            (Server::Gateway, &["--config", "--dev-keys"][..]),
            (
                Server::Gateway,
                &["--config", "one.toml", "--config", "two.toml"][..],
            ),
            (Server::Gateway, &["--unknown"][..]),
        ] {
            assert!(
                StartupOptions::parse(server, args.iter().copied().map(OsString::from)).is_err(),
                "{args:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn invalid_pool_bounds_are_rejected() {
        let mut cfg = Config::default();
        cfg.live_search.pool_min_connections = 21;
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn connection_budget_summary_format() {
        let cfg = LiveSearchConfig::default();
        let summary = cfg.connection_budget_summary();
        assert!(summary.contains("max_connections=20"));
        assert!(summary.contains("min_connections=2"));
        assert!(summary.contains("acquire_timeout=5s"));
        assert!(summary.contains("idle_timeout=600s"));
        assert!(summary.contains("max_lifetime=1800s"));
    }
}
