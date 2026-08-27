//! OpenTelemetry tracing initialization for the gateway.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TelemetryError {
    #[error("OTLP exporter error: {0}")]
    Otlp(String),
    #[error("subscriber registration error: {0}")]
    Subscriber(String),
}

/// Initialize the tracing stack with W3C propagation and OTLP/HTTP export.
///
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is honored directly. The fallback is the
/// conventional OTLP/HTTP endpoint on port 4318; 4317 is reserved for OTLP
/// gRPC in the standard defaults.
///
/// The Axum router separately installs `OtelAxumLayer` when this feature is
/// active so incoming `traceparent`/`tracestate` headers are actually extracted.
///
/// # Errors
/// Returns [`TelemetryError`] if exporter or subscriber setup fails.
pub fn init_telemetry() -> Result<SdkTracerProvider, TelemetryError> {
    use opentelemetry::global;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Registry};

    global::set_text_map_propagator(TraceContextPropagator::new());

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
