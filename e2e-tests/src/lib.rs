//! e2e test helpers — unit-tested in this crate's lib module.

/// Resolve the base URL — use `BASE_URL` env var or fall back to `http://localhost:3000`.
///
/// When `override_url` is `Some`, that value is used directly instead of
/// consulting the environment.  This is useful in unit tests to avoid unsafe
/// `env::set_var` / `env::remove_var` calls.
#[must_use]
pub fn base_url(override_url: Option<&str>) -> String {
    override_url
        .map(String::from)
        .or_else(|| std::env::var("BASE_URL").ok())
        .unwrap_or_else(|| "http://localhost:3000".to_string())
}

/// Build a URL from a base and a path segment.  Handles leading slashes on
/// `path` and trailing slashes on `base`.
#[must_use]
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // base_url()
    // ---------------------------------------------------------------------------

    #[test]
    fn test_base_url_default() {
        let url = base_url(None);
        assert!(!url.is_empty(), "base_url(None) should not be empty");
        assert!(
            url.starts_with("http://") || url.starts_with("https://"),
            "base_url(None) should return a valid HTTP URL, got: {url}"
        );
    }

    #[test]
    fn test_base_url_explicit_override() {
        let url = base_url(Some("http://example.com:8080"));
        assert_eq!(url, "http://example.com:8080");
    }

    // ---------------------------------------------------------------------------
    // join_url() — URL composition
    // ---------------------------------------------------------------------------

    #[test]
    fn test_join_url_basic() {
        let url = join_url("http://localhost:3000", "/api/health");
        assert_eq!(url, "http://localhost:3000/api/health");
    }

    #[test]
    fn test_join_url_no_trailing_slash_on_base() {
        let url = join_url("http://localhost:3000", "api/health");
        assert_eq!(url, "http://localhost:3000/api/health");
    }

    #[test]
    fn test_join_url_trailing_slash_on_base() {
        let url = join_url("http://localhost:3000/", "/api/health");
        assert_eq!(url, "http://localhost:3000/api/health");
    }

    #[test]
    fn test_join_url_empty_path() {
        let url = join_url("http://localhost:3000", "");
        assert_eq!(url, "http://localhost:3000/");
    }
}
