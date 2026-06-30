//! OpenTelemetry initialisation for the `live-search` crate.
//!
//! Provides [`init_telemetry`] which builds a layered `tracing` subscriber
//! with `EnvFilter`, `fmt`, and an OTel layer, and returns the
//! [`SdkTracerProvider`] so the caller can `force_flush` / `shutdown` it
//! gracefully.
//!
//! W3C `traceparent` / `tracestate` propagation is configured via
//! [`opentelemetry::global::set_text_map_propagator`].
//!
//! # Feature gate
//! This module is compiled only when the `otel` feature is active
//! (gated at `lib.rs` level).
//!
//! # dev-tools feature note
//! The `dev-tools` feature (behind `RUSTFLAGS="--cfg tokio_unstable"`) is
//! independent; this module works with or without it.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Errors that can occur during telemetry initialisation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// OpenTelemetry exporter construction failed.
    #[error("OTLP exporter setup failed: {0}")]
    Otlp(String),
    /// Tracing subscriber initialisation failed (e.g. duplicate init).
    #[error("tracing subscriber initialisation failed: {0}")]
    Subscriber(String),
    /// Environment filter directive failed to parse.
    #[error("invalid filter directive: {0}")]
    Filter(String),
}

/// Initialise the tracing subscriber with an OTel layer.
///
/// Returns the [`SdkTracerProvider`] so the caller can perform
/// [`SdkTracerProvider::force_flush`] and
/// [`SdkTracerProvider::shutdown`] during graceful shutdown.
///
/// # Errors
///
/// Returns [`TelemetryError`] if the OTLP exporter could not be built,
/// the subscriber could not be initialised, or a filter directive was
/// malformed.
#[must_use]
pub fn init_telemetry() -> Result<SdkTracerProvider, TelemetryError> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4317".to_string());

    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => return Err(TelemetryError::Otlp(e.to_string())),
    };

    let resource = Resource::builder()
        .with_service_name(env!("CARGO_CRATE_NAME"))
        .build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("live-search");
    opentelemetry::global::set_tracer_provider(provider.clone());

    // W3C traceparent / tracestate propagation for distributed tracing
    // interop (Jaeger, Tempo, Honeycomb, etc.).
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
