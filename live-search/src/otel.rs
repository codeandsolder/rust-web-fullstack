//! OpenTelemetry initialisation for the `live-search` crate.
//!
//! Provides [`init_telemetry`] which builds a layered `tracing` subscriber
//! with `EnvFilter`, `fmt`, and an `OTel` layer, and returns the
//! [`SdkTracerProvider`] so the caller can `force_flush` / `shutdown` it
//! gracefully.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TelemetryError {
    #[error("OTLP exporter setup failed: {0}")]
    Otlp(String),
    #[error("tracing subscriber initialisation failed: {0}")]
    Subscriber(String),
    #[error("invalid filter directive: {0}")]
    Filter(String),
}

/// Initialise the tracing subscriber with an OTLP/HTTP exporter.
///
/// `OTEL_EXPORTER_OTLP_ENDPOINT` follows the standard OpenTelemetry setting.
/// When absent, OTLP/HTTP uses port 4318; port 4317 is the conventional OTLP
/// gRPC endpoint and was the wrong default for `.with_http()`.
///
/// # Errors
/// Returns [`TelemetryError`] when exporter/subscriber/filter setup fails.
pub fn init_telemetry() -> Result<SdkTracerProvider, TelemetryError> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4318".to_string());

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| TelemetryError::Otlp(e.to_string()))?;

    let resource = Resource::builder()
        .with_service_name(env!("CARGO_CRATE_NAME"))
        .build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("live-search");
    opentelemetry::global::set_tracer_provider(provider.clone());
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let filter =
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,live_search=debug,tower_http=debug"))
            .add_directive("h2=warn".parse().map_err(
                |e: tracing_subscriber::filter::ParseError| TelemetryError::Filter(e.to_string()),
            )?)
            .add_directive("primp_h2=warn".parse().map_err(
                |e: tracing_subscriber::filter::ParseError| TelemetryError::Filter(e.to_string()),
            )?)
            .add_directive("http2=info".parse().map_err(
                |e: tracing_subscriber::filter::ParseError| TelemetryError::Filter(e.to_string()),
            )?);

    let fmt_layer = tracing_subscriber::fmt::layer().with_target(true).compact();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()
        .map_err(|e| TelemetryError::Subscriber(e.to_string()))?;

    Ok(provider)
}
