//! Shared OpenTelemetry/HTTP tracing setup for the server-side examples.
//!
//! Service identity and log filtering are supplied by each binary; exporter
//! configuration, W3C propagation and subscriber installation are common.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
pub use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Service-specific tracing settings.
#[derive(Debug, Clone, Copy)]
pub struct TelemetryConfig {
    /// Exported resource name and tracer instrumentation name.
    pub service_name: &'static str,
    /// Filter to use when RUST_LOG is absent or invalid.
    pub default_filter: &'static str,
    /// Additional directives that take precedence over RUST_LOG.
    pub directives: &'static [&'static str],
}

/// Telemetry startup failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TelemetryError {
    #[error("OTLP exporter setup failed: {0}")]
    Otlp(String),
    #[error("invalid tracing directive: {0}")]
    Filter(#[from] tracing_subscriber::filter::ParseError),
    #[error("subscriber registration failed: {0}")]
    Subscriber(String),
}

fn build_filter(config: TelemetryConfig) -> Result<EnvFilter, TelemetryError> {
    let mut filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(config.default_filter));
    for directive in config.directives {
        filter = filter.add_directive(directive.parse()?);
    }
    Ok(filter)
}

/// Install a compact tracing subscriber and an OTLP/HTTP span exporter.
///
/// Honors OTEL_EXPORTER_OTLP_ENDPOINT, defaulting to the standard HTTP
/// endpoint on port 4318. W3C propagation and the global tracer provider
/// are only installed after the subscriber successfully registers.
///
/// # Errors
/// Returns an error for invalid filter directives, exporter setup failures,
/// or if another tracing subscriber has already been installed.
pub fn init_telemetry(config: TelemetryConfig) -> Result<SdkTracerProvider, TelemetryError> {
    let filter = build_filter(config)?;
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4318".to_string());
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| TelemetryError::Otlp(error.to_string()))?;

    let resource = Resource::builder()
        .with_service_name(config.service_name)
        .build();
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();
    let tracer = provider.tracer(config.service_name);

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(true).compact())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()
        .map_err(|error| TelemetryError::Subscriber(error.to_string()))?;

    opentelemetry::global::set_tracer_provider(provider.clone());
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_directives_override_the_fallback_filter() -> Result<(), TelemetryError> {
        let config = TelemetryConfig {
            service_name: "test-service",
            default_filter: "info,h2=debug",
            directives: &["h2=warn", "http2=info"],
        };
        // Build without registering global subscriber/exporter state.
        let filter = build_filter(config)?;
        let display = filter.to_string();
        assert!(display.contains("h2=warn"));
        assert!(display.contains("http2=info"));
        Ok(())
    }

    #[test]
    fn malformed_directive_returns_error() {
        let config = TelemetryConfig {
            service_name: "test-service",
            default_filter: "info",
            directives: &["not[valid"],
        };
        assert!(matches!(
            build_filter(config),
            Err(TelemetryError::Filter(_))
        ));
    }
}
