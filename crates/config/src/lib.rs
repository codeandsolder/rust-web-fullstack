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
    use std::sync::Mutex;
    use std::sync::OnceLock;

    use super::*;

    /// Process-wide serialisation mutex for env-mutating tests.
    ///
    /// Tests in a single binary run on multiple threads by default
    /// (`cargo test --test-threads = N CPU cores`). Without this lock,
    /// the env-var-mutating test races with `defaults_match_documented_values`
    /// (which reads `RWF_CONFIG` indirectly) and produces ~20% flaky
    /// failures. Acquire the lock at the top of every test that touches
    /// the process env. `PoisonError::into_inner` is used so a panic in
    /// one test doesn't lock out the rest.
    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Temporarily set `key` to `value` for the duration of `f`, then
    /// restore the previous value (or remove the var if it was unset).
    ///
    /// SAFETY: requires the caller to hold [`env_test_lock`] for the
    /// duration of the closure so concurrent tests don't observe the
    /// mutation. Tests within a single `cargo test` binary run on
    /// multiple threads by default.
    fn with_env_var<F, R>(key: &str, value: &str, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let original = std::env::var(key).ok();
        // SAFETY: process-env mutation is safe because the env_test_lock
        // serialises every test that calls this helper.
        unsafe {
            std::env::set_var(key, value);
        }
        let result = f();
        match original {
            Some(v) => unsafe {
                std::env::set_var(key, v);
            },
            None => unsafe {
                std::env::remove_var(key);
            },
        }
        result
    }

    #[test]
    fn defaults_match_documented_values() {
        // The env-mutating tests acquire `env_test_lock`; this one
        // does not need to touch the env but DOES depend on
        // `RWF_CONFIG` being unset (the loader returns ConfigPathNotFound
        // if it points to a missing file). Hold the lock so we read a
        // deterministic env state.
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cfg = Config::load().expect("load should succeed with defaults");
        assert_eq!(cfg.gateway.port, 3001);
        assert_eq!(cfg.live_search.port, 3000);
        assert_eq!(cfg.live_search.pool_max_connections, 20);
        assert_eq!(cfg.live_search.pool_min_connections, 2);
        assert_eq!(cfg.live_search.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.sse_broadcast_buffer, 256);
        assert_eq!(cfg.gateway.refresh_token_ttl_secs, 60 * 60 * 24 * 30);
    }

    /// `RWF_CONFIG=/nonexistent.toml` produces a `ConfigPathNotFound` error
    /// (not a silent fallback).
    #[test]
    fn rwf_config_missing_path_errors() {
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = with_env_var("RWF_CONFIG", "/this/path/does/not/exist.toml", Config::load);
        assert!(
            matches!(result, Err(ConfigError::ConfigPathNotFound(_))),
            "expected ConfigPathNotFound for nonexistent RWF_CONFIG, got {result:?}",
        );
    }

    /// `connection_budget_summary` round-trips the configured pool values
    /// in a single human-readable line (used by `live-search::bootstrap`).
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
