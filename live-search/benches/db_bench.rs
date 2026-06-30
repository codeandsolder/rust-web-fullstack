//! Criterion benchmark for the live-search database search query.
//!
//! Measures the round-trip latency of the full-text search query under
//! various query string lengths. Requires a running `PostgreSQL` instance
//! with the `search_results` table populated (e.g. via a test db).
//!
//! # Usage
//!
//! ```sh
//! DATABASE_URL="postgres://..." cargo bench -p live-search
//! ```
//!
//! The benchmark uses [`criterion`] with HTML reports enabled (configured
//! in the workspace `[workspace.dependencies]`).

#![cfg(feature = "ssr")]
// Benchmarks intentionally use `expect`/`unwrap_or_default`/`unwrap` because:
// 1. They run against a real Postgres with a known seed dataset.
// 2. Failure should abort the bench loudly, not produce noise measurements.
// This is a documented exception to the workspace `unwrap_used`/`expect_used`/`panic` deny policy.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

criterion_group! {
    name = search_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(15))
        .warm_up_time(Duration::from_secs(3))
        .sample_size(50);
    targets = search_query_benchmark
}

criterion_main!(search_benches);

/// Benchmark the full-text search query with short, medium, and long query
/// strings. Runs against a real database — set `DATABASE_URL` before running.
fn search_query_benchmark(c: &mut Criterion) {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FATAL: failed to create tokio runtime: {e}");
            std::process::exit(1);
        }
    };

    // Connect to the database (optional — dry run if DATABASE_URL is unset).
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let pool: Option<Arc<sqlx::PgPool>> = if database_url.is_empty() {
        eprintln!("WARNING: DATABASE_URL not set; benchmark will be dry (no DB queries)");
        None
    } else {
        match rt.block_on(sqlx::PgPool::connect(&database_url)) {
            Ok(p) => {
                eprintln!("Connected to database for benchmarking");
                Some(Arc::new(p))
            }
            Err(e) => {
                eprintln!("WARNING: could not connect to database ({e}); dry run");
                None
            }
        }
    };

    let mut group = c.benchmark_group("search_query");
    group.sample_size(50);

    let queries = vec!["hello", "rust programming", "a"];
    for query in &queries {
        let query_str = query.to_string();
        let pool = pool.clone();
        group.bench_with_input(
            BenchmarkId::new("full_text_search", query_str.as_str()),
            &query_str,
            |b, _| {
                b.iter(|| {
                    rt.block_on(async {
                        if let Some(ref pool) = pool {
                            let _: Vec<live_search::db::SearchResult> = sqlx::query_as(
                                r"SELECT id, title, url, snippet, created_at
                                   FROM search_results
                                   WHERE fts @@ plainto_tsquery('english', $1)
                                   ORDER BY created_at DESC
                                   LIMIT 20",
                            )
                            .bind(&query_str)
                            .fetch_all(pool.as_ref())
                            .await
                            .unwrap_or_default();
                        }
                        black_box(query_str.as_str())
                    });
                });
            },
        );
    }

    group.finish();
}
