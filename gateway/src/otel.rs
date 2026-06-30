//! OpenTelemetry tracing initialization.
//!
//! Provides [`init_telemetry`] which constructs a
//! [`SdkTracerProvider`] with an OTLP HTTP/protobuf exporter, configures a
//! layered tracing subscriber (`Registry` + `EnvFilter` + `fmt` + `OTel`), and
//! sets the W3C `TraceContextPropagator` for distributed trace propagation.
//!
//! # Shutdown
//!
//! The returned [`SdkTracerProvider`] must be force-flushed and shut down
//! before the process exits (see rust-tracing §3.2). Failure to do so **will**
//! drop in-flight spans.
//!
//! ```rust,ignore
//! let provider = otel::init_telemetry()?;
//! // ... run app ...
//! let _ = tokio::time::timeout(Duration::from_secs(5), provider.force_flush()).await;
//! let _ = tokio::time::timeout(Duration::from_secs(5), provider.shutdown()).await;
//! ```

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use thiserror::Error;

/// Errors that can occur during telemetry initialization.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// OTLP exporter construction failed.
    #[error("OTLP exporter error: {0}")]
    Otlp(String),
    /// Tracing subscriber registration failed.
    #[error("subscriber registration error: {0}")]
    Subscriber(String),
}

/// Initialize the OpenTelemetry tracing stack.
///
/// Configures:
/// * W3C `TraceContextPropagator` for `traceparent` / `tracestate` interop
/// * An OTLP HTTP/protobuf `SpanExporter` pointing at
///   `http://127.0.0.1:4318/otlp/v1/traces`
/// * A layered subscriber: `Registry` ← `EnvFilter` ← `fmt` ← `OTel`
///   (per rust-tracing §3 canonical order)
///
/// # Errors
///
/// Returns [`TelemetryError`] if the OTLP exporter cannot be built or the
/// subscriber cannot be registered.
pub fn init_telemetry() -> Result<SdkTracerProvider, TelemetryError> {
    use opentelemetry::global;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Registry};

    // W3C traceparent / tracestate propagator (§6.3)
    global::set_text_map_propagator(TraceContextPropagator::new());

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint("http://127.0.0.1:4318/otlp/v1/traces")
        .build()
        .map_err(|e| TelemetryError::Otlp(e.to_string()))?;

    let resource = Resource::builder()
        .with_service_name(env!("CARGO_CRATE_NAME"))
        .build();

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("gateway-example");
    global::set_tracer_provider(provider.clone());

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("gateway_example=info,tower_http=debug"));

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    Registry::default()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(true).compact())
        .with(otel_layer)
        .try_init()
        .map_err(|e| TelemetryError::Subscriber(e.to_string()))?;

    Ok(provider)
}
