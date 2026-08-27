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
    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_lossless,
        reason = "documented defaults are small fixed values"
    )]
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = std::env::var("RWF_CONFIG").ok();
        if let Some(path) = config_path.as_deref()
            && !Path::new(path).exists()
        {
            return Err(ConfigError::ConfigPathNotFound(path.to_string()));
        }

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
                default_gateway_sse_broadcast_buffer() as i64,
            )?
            .set_default(
                "gateway.refresh_token_ttl_secs",
                default_gateway_refresh_token_ttl_secs() as i64,
            )?
            .set_default(
                "gateway.access_token_ttl_secs",
                default_gateway_access_token_ttl_secs() as i64,
            )?
            .set_default("live_search.port", 3000_i64)?
            .set_default(
                "live_search.database_url",
                "postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo",
            )?
            .set_default(
                "live_search.pool_max_connections",
                default_pool_max_connections() as i64,
            )?
            .set_default(
                "live_search.pool_min_connections",
                default_pool_min_connections() as i64,
            )?
            .set_default(
                "live_search.pool_acquire_timeout_secs",
                default_pool_acquire_timeout_secs() as i64,
            )?
            .set_default(
                "live_search.pool_idle_timeout_secs",
                default_pool_idle_timeout_secs() as i64,
            )?
            .set_default(
                "live_search.pool_max_lifetime_secs",
                default_pool_max_lifetime_secs() as i64,
            )?
            .set_default(
                "live_search.sse_broadcast_buffer",
                default_live_search_sse_broadcast_buffer() as i64,
            )?
            .add_source(
                config::File::new(
                    config_path.as_deref().unwrap_or("config.toml"),
                    config::FileFormat::Toml,
                )
                .required(false),
            )
            .add_source(
                config::Environment::with_prefix("RWF")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            );

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
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: String,
        original: Option<String>,
    }

    impl EnvVarGuard {
        #[expect(
            unsafe_code,
            reason = "process environment mutation is serialized by env_test_lock"
        )]
        fn set(key: &str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            unsafe { std::env::set_var(key, value) };
            Self {
                key: key.to_string(),
                original,
            }
        }
    }

    impl Drop for EnvVarGuard {
        #[expect(
            unsafe_code,
            reason = "process environment mutation is serialized by env_test_lock"
        )]
        fn drop(&mut self) {
            match self.original.as_deref() {
                Some(value) => unsafe { std::env::set_var(&self.key, value) },
                None => unsafe { std::env::remove_var(&self.key) },
            }
        }
    }

    fn with_env_var<F, R>(key: &str, value: &str, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let _guard = EnvVarGuard::set(key, value);
        f()
    }

    #[test]
    fn defaults_match_documented_values() -> Result<(), ConfigError> {
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cfg = Config::load()?;
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
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = with_env_var("RWF_CONFIG", "/this/path/does/not/exist.toml", Config::load);
        assert!(matches!(result, Err(ConfigError::ConfigPathNotFound(_))));
    }

    #[test]
    fn invalid_zero_sse_buffer_is_rejected() {
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = with_env_var("RWF_LIVE_SEARCH__SSE_BROADCAST_BUFFER", "0", Config::load);
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
