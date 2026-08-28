#![cfg(feature = "otel-tests")]

use anyhow::{Context, Result};
use opentelemetry::trace::{SpanKind, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
use tracing::Instrument as _;
use tracing_subscriber::layer::SubscriberExt as _;

use e2e_tests::common::TestEnv;

/// Verify the optional SQLx instrumentation emits a database client span below
/// the active request/tracing span instead of creating an orphan root span.
#[tokio::test]
async fn sqlx_span_is_child_of_tracing_parent() -> Result<()> {
    let exporter = InMemorySpanExporterBuilder::new().build();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());

    let tracer = provider.tracer("live-search-otel-test");
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(tracer));
    let dispatch = tracing::Dispatch::new(subscriber);
    let _dispatch_guard = tracing::dispatcher::set_default(&dispatch);

    let db = TestEnv::postgres().await?;
    let app_pool = live_search::state::app_pool_from_raw(db.pool().clone());

    let request_span = tracing::info_span!("otel_db_test_request");
    let value = async {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&app_pool)
            .await
    }
    .instrument(request_span.clone())
    .await?;
    assert_eq!(value, 1);

    // The tracing parent is exported when its final handle is dropped.
    drop(request_span);
    provider
        .force_flush()
        .context("failed to flush test OpenTelemetry spans")?;

    let spans = exporter
        .get_finished_spans()
        .context("failed to read exported OpenTelemetry spans")?;
    let parent = spans
        .iter()
        .find(|span| span.name.as_ref() == "otel_db_test_request")
        .context("request tracing span was not exported")?;
    let db_span = spans
        .iter()
        .find(|span| span.span_kind == SpanKind::Client)
        .context("sqlx-otel did not emit a database client span")?;

    assert_eq!(db_span.span_context.trace_id(), parent.span_context.trace_id());
    assert_eq!(db_span.parent_span_id, parent.span_context.span_id());

    Ok(())
}
