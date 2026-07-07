//! Workspace configuration loaded from `config.toml` with
//! `RWF_*` environment variable overrides.
//!
//! Each binary in the workspace calls [`Config::load`] at startup. The
//! loader reads `config.toml` from the current working directory (or the
//! path in `RWF_CONFIG`), then layers any matching `RWF_*` environment
//! variables on top. Missing required keys produce a clear error.
//!
//! This replaces the previous per-binary ad-hoc `std::env::var` calls,
//! eliminating the need for hand-rolled fallback logic.

use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

/// Top-level workspace config.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub live_search: LiveSearchConfig,
    #[serde(default)]
    pub otel: OtelConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub proxy_upstream_url: String,
    pub cors: CorsConfig,
    pub session: SessionConfig,
    /// Buffer size for the gateway's SSE broadcast channel.
    /// Overridable via `RWF_GATEWAY__SSE_BROADCAST_BUFFER`.
    #[serde(default = "default_gateway_sse_broadcast_buffer")]
    pub sse_broadcast_buffer: usize,
    /// Lifetime of newly-issued refresh tokens, in seconds.
    /// Overridable via `RWF_GATEWAY__REFRESH_TOKEN_TTL_SECS`.
    #[serde(default = "default_gateway_refresh_token_ttl_secs")]
    pub refresh_token_ttl_secs: u64,
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
        }
    }
}

const fn default_gateway_sse_broadcast_buffer() -> usize {
    256
}

const fn default_gateway_refresh_token_ttl_secs() -> u64 {
    // 30 days, matching the previous hand-rolled constant in
    // `gateway/src/auth/refresh.rs::REFRESH_TOKEN_TTL_SECONDS`.
    60 * 60 * 24 * 30
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
    /// Pool hardening tunables — all overridable via
    /// `RWF_LIVE_SEARCH__POOL_*` (see the documented example below).
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
    /// Buffer size for the live-search SSE broadcast channel.
    /// Overridable via `RWF_LIVE_SEARCH__SSE_BROADCAST_BUFFER`.
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
    /// Human-readable connection-budget summary used by `db::create_pool`
    /// and emitted at startup so operators can spot
    /// `pool.max_connections + 1 (PgListener) > pg.max_connections`
    /// violations at a glance.
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OtelConfig {
    pub endpoint: Option<String>,
}

/// Errors that can occur when loading the configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The underlying `config` crate returned an error (file not found,
    /// parse error, missing required key, etc.).
    #[error("config load failed: {0}")]
    Load(#[from] config::ConfigError),
    /// The required `RWF_CONFIG` path was set but does not exist.
    #[error("RWF_CONFIG path {0} does not exist")]
    ConfigPathNotFound(String),
}

impl Config {
    /// Load the configuration from `config.toml` (or `RWF_CONFIG`),
    /// with `RWF_*` environment variable overrides.
    ///
    /// Resolution order:
    /// 1. `RWF_CONFIG` env var (if set) — path to a TOML file
    /// 2. `./config.toml` (relative to CWD)
    /// 3. Defaults baked into the structs
    ///
    /// Environment variables are layered with the `RWF_` prefix and
    /// `__` as the section separator. For example:
    /// - `RWF_GATEWAY__PORT=4000` overrides `gateway.port`
    /// - `RWF_GATEWAY__SSE_BROADCAST_BUFFER=512` overrides
    ///   `gateway.sse_broadcast_buffer`
    /// - `RWF_LIVE_SEARCH__DATABASE_URL=...` overrides
    ///   `live_search.database_url`
    /// - `RWF_LIVE_SEARCH__POOL_MAX_CONNECTIONS=50` overrides
    ///   `live_search.pool_max_connections`
    /// - `RWF_LIVE_SEARCH__SSE_BROADCAST_BUFFER=512` overrides
    ///   `live_search.sse_broadcast_buffer`
    ///
    /// # Errors
    /// Returns [`ConfigError`] if the file is unreadable or unparseable,
    /// or if a required key is missing.
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = std::env::var("RWF_CONFIG").ok();

        let builder = config::Config::builder()
            // Hard-coded defaults — kept in sync with the `Default`
            // impls above.
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
            .set_default("otel.endpoint", "")?
            // Layer 2: TOML file (if present).
            .add_source(
                config::File::with_name(config_path.as_deref().unwrap_or("config.toml"))
                    .required(false),
            )
            // Layer 3: RWF_* env vars (highest priority).
            .add_source(
                config::Environment::with_prefix("RWF")
                    .separator("__")
                    .try_parsing(true),
            );

        let cfg: Self = builder.build()?.try_deserialize()?;

        if let Some(path) = config_path.as_deref()
            && !Path::new(path).exists()
        {
            return Err(ConfigError::ConfigPathNotFound(path.to_string()));
        }

        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        // Defaults load even when config.toml is absent.
        let cfg = Config::load().expect("load should succeed with defaults");
        assert_eq!(cfg.gateway.port, 3001);
        assert_eq!(cfg.live_search.port, 3000);
        assert_eq!(cfg.live_search.pool_max_connections, 20);
        assert_eq!(cfg.live_search.pool_min_connections, 2);
        assert_eq!(cfg.live_search.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.refresh_token_ttl_secs, 60 * 60 * 24 * 30);
    }
}
