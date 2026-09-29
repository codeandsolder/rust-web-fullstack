//! Typed workspace configuration loaded from `config.toml` plus `RWF_*`
//! environment overrides.

use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

const MAX_I64_AS_U64: u64 = 9_223_372_036_854_775_807;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub live_search: LiveSearchConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub proxy_upstream_url: String,
    pub cors: CorsConfig,
    pub session: SessionConfig,
    #[serde(default = "default_gateway_sse_broadcast_buffer")]
    pub sse_broadcast_buffer: usize,
    #[serde(default = "default_gateway_refresh_token_ttl_secs")]
    pub refresh_token_ttl_secs: u64,
    #[serde(default = "default_gateway_access_token_ttl_secs")]
    pub access_token_ttl_secs: u64,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 3001,
            proxy_upstream_url: "https://ipapi.co".to_string(),
            cors: CorsConfig::default(),
            session: SessionConfig::default(),
            sse_broadcast_buffer: default_gateway_sse_broadcast_buffer(),
            refresh_token_ttl_secs: default_gateway_refresh_token_ttl_secs(),
            access_token_ttl_secs: default_gateway_access_token_ttl_secs(),
        }
    }
}

const fn default_gateway_sse_broadcast_buffer() -> usize {
    256
}
const fn default_gateway_refresh_token_ttl_secs() -> u64 {
    60 * 60 * 24 * 30
}
const fn default_gateway_access_token_ttl_secs() -> u64 {
    15 * 60
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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
pub struct LiveSearchConfig {
    pub port: u16,
    pub database_url: String,
    #[serde(default = "default_pool_max_connections")]
    pub pool_max_connections: u32,
    #[serde(default = "default_pool_min_connections")]
    pub pool_min_connections: u32,
    #[serde(default = "default_pool_acquire_timeout_secs")]
    pub pool_acquire_timeout_secs: u64,
    #[serde(default = "default_pool_idle_timeout_secs")]
    pub pool_idle_timeout_secs: u64,
    #[serde(default = "default_pool_max_lifetime_secs")]
    pub pool_max_lifetime_secs: u64,
    #[serde(default = "default_live_search_sse_broadcast_buffer")]
    pub sse_broadcast_buffer: usize,
}

impl Default for LiveSearchConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            database_url: "postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo".to_string(),
            pool_max_connections: default_pool_max_connections(),
            pool_min_connections: default_pool_min_connections(),
            pool_acquire_timeout_secs: default_pool_acquire_timeout_secs(),
            pool_idle_timeout_secs: default_pool_idle_timeout_secs(),
            pool_max_lifetime_secs: default_pool_max_lifetime_secs(),
            sse_broadcast_buffer: default_live_search_sse_broadcast_buffer(),
        }
    }
}

impl LiveSearchConfig {
    #[must_use]
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

const fn default_pool_max_connections() -> u32 {
    20
}
const fn default_pool_min_connections() -> u32 {
    2
}
const fn default_pool_acquire_timeout_secs() -> u64 {
    5
}
const fn default_pool_idle_timeout_secs() -> u64 {
    600
}
const fn default_pool_max_lifetime_secs() -> u64 {
    1800
}
const fn default_live_search_sse_broadcast_buffer() -> usize {
    256
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config load failed: {0}")]
    Load(#[from] config::ConfigError),
    #[error("RWF_CONFIG path {0} does not exist")]
    ConfigPathNotFound(String),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

fn i64_default<T>(value: T, name: &str) -> Result<i64, ConfigError>
where
    i64: TryFrom<T>,
{
    i64::try_from(value)
        .map_err(|_| ConfigError::Invalid(format!("{name} default does not fit in i64")))
}

impl Config {
    /// Load defaults, an optional TOML file, then `RWF_*` environment
    /// overrides (`__` separates nested keys), and validate cross-field
    /// invariants before returning.
    ///
    /// Example: `RWF_LIVE_SEARCH__POOL_MAX_CONNECTIONS=50`.
    ///
    /// # Errors
    /// Returns [`ConfigError`] for missing explicit files, parse/deserialization
    /// failures, or invalid values/invariants.
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = std::env::var("RWF_CONFIG").ok();
        Self::load_from(config_path.as_deref(), None)
    }

    fn load_from(
        config_path: Option<&str>,
        environment_source: Option<config::Map<String, String>>,
    ) -> Result<Self, ConfigError> {
        if let Some(path) = config_path
            && !Path::new(path).exists()
        {
            return Err(ConfigError::ConfigPathNotFound(path.to_string()));
        }

        let environment = config::Environment::with_prefix("RWF")
            .prefix_separator("_")
            .separator("__")
            .try_parsing(true)
            .source(environment_source);

        let builder = config::Config::builder()
            .set_default("gateway.port", 3001_i64)?
            .set_default("gateway.proxy_upstream_url", "https://ipapi.co")?
            .set_default(
                "gateway.cors.allowed_origins",
                "http://localhost:3000,http://localhost:3001,http://localhost:3002",
            )?
            .set_default("gateway.session.cookie_secure", true)?
            .set_default(
                "gateway.sse_broadcast_buffer",
                i64_default(
                    default_gateway_sse_broadcast_buffer(),
                    "gateway.sse_broadcast_buffer",
                )?,
            )?
            .set_default(
                "gateway.refresh_token_ttl_secs",
                i64_default(
                    default_gateway_refresh_token_ttl_secs(),
                    "gateway.refresh_token_ttl_secs",
                )?,
            )?
            .set_default(
                "gateway.access_token_ttl_secs",
                i64_default(
                    default_gateway_access_token_ttl_secs(),
                    "gateway.access_token_ttl_secs",
                )?,
            )?
            .set_default("live_search.port", 3000_i64)?
            .set_default(
                "live_search.database_url",
                "postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo",
            )?
            .set_default(
                "live_search.pool_max_connections",
                i64::from(default_pool_max_connections()),
            )?
            .set_default(
                "live_search.pool_min_connections",
                i64::from(default_pool_min_connections()),
            )?
            .set_default(
                "live_search.pool_acquire_timeout_secs",
                i64_default(
                    default_pool_acquire_timeout_secs(),
                    "live_search.pool_acquire_timeout_secs",
                )?,
            )?
            .set_default(
                "live_search.pool_idle_timeout_secs",
                i64_default(
                    default_pool_idle_timeout_secs(),
                    "live_search.pool_idle_timeout_secs",
                )?,
            )?
            .set_default(
                "live_search.pool_max_lifetime_secs",
                i64_default(
                    default_pool_max_lifetime_secs(),
                    "live_search.pool_max_lifetime_secs",
                )?,
            )?
            .set_default(
                "live_search.sse_broadcast_buffer",
                i64_default(
                    default_live_search_sse_broadcast_buffer(),
                    "live_search.sse_broadcast_buffer",
                )?,
            )?
            .add_source(
                config::File::new(
                    config_path.unwrap_or("config.toml"),
                    config::FileFormat::Toml,
                )
                .required(false),
            )
            .add_source(environment);

        let cfg: Self = builder.build()?.try_deserialize()?;
        cfg.validate()?;
        Ok(cfg)
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

        if self.live_search.database_url.trim().is_empty() {
            return invalid("live_search.database_url must not be empty");
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

    fn empty_environment() -> config::Map<String, String> {
        config::Map::new()
    }

    fn environment(entries: &[(&str, &str)]) -> config::Map<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn defaults_match_documented_values() -> Result<(), ConfigError> {
        let cfg = Config::load_from(None, Some(empty_environment()))?;
        assert_eq!(cfg.gateway.port, 3001);
        assert_eq!(cfg.live_search.port, 3000);
        assert_eq!(cfg.live_search.pool_max_connections, 20);
        assert_eq!(cfg.live_search.pool_min_connections, 2);
        assert_eq!(cfg.live_search.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.refresh_token_ttl_secs, 60 * 60 * 24 * 30);
        assert_eq!(cfg.gateway.access_token_ttl_secs, 15 * 60);
        Ok(())
    }

    #[test]
    fn rwf_config_missing_path_errors() {
        let result = Config::load_from(
            Some("/this/path/does/not/exist.toml"),
            Some(empty_environment()),
        );
        assert!(matches!(result, Err(ConfigError::ConfigPathNotFound(_))));
    }

    #[test]
    fn invalid_zero_sse_buffer_is_rejected() {
        let result = Config::load_from(
            None,
            Some(environment(&[(
                "RWF_LIVE_SEARCH__SSE_BROADCAST_BUFFER",
                "0",
            )])),
        );
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
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
