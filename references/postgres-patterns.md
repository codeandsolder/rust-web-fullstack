# PostgreSQL + sqlx Patterns Reference

## Table of Contents
1. [Connection Pooling](#1-connection-pooling)
2. [JSONB Storage](#2-jsonb-storage)
3. [Full-Text Search](#3-full-text-search)
4. [LISTEN/NOTIFY Live Queries](#4-listennotify-live-queries)
5. [Migrations](#5-migrations)
6. [Compile-Time Query Checking](#6-compile-time-query-checking)
7. [Error Handling](#7-error-handling)
8. [Transactions](#8-transactions)
9. [Performance Tuning (512MB Config)](#9-performance-tuning-512mb-config)
10. [Multi-Process Access](#10-multi-process-access)
11. [TTL (Time-To-Live)](#11-ttl-time-to-live)
12. [Pitfalls](#12-pitfalls)

---

## 1. Connection Pooling

### Pool Setup

```rust
use sqlx::postgres::{PgPool, PgPoolOptions};

let pool = PgPoolOptions::new()
    .max_connections(20)           // hard cap, pool blocks when all checked out
    .min_connections(0)            // pre-warm connections, maintained best-effort
    .acquire_timeout(Duration::from_secs(5))
    .idle_timeout(Duration::from_secs(10 * 60))   // 10 min
    .max_lifetime(Duration::from_secs(30 * 60))   // 30 min
    .test_before_acquire(true)     // ping before handing out connection
    .connect("postgresql://localhost/mydb")
    .await?;
```

### Pool Options Reference

| Option | Default | Purpose |
|--------|---------|---------|
| `max_connections` | 10 | Hard cap. Will block `acquire()` when all in use |
| `min_connections` | 0 | Pre-warmed connections in background |
| `acquire_timeout` | 30s | Total time for semaphore + health check + connection open |
| `max_lifetime` | 30 min | Close and replace connections older than this |
| `idle_timeout` | 10 min | Reap idle connections older than this |
| `test_before_acquire` | true | Ping before handing to caller |
| `fair` | true | First-come-first-serve ordering |

### Lifecycle Hooks

```rust
let pool = PgPoolOptions::new()
    .after_connect(|conn, _meta| Box::pin(async move {
        conn.execute("SET application_name = 'my_service'").await?;
        Ok(())
    }))
    .before_acquire(|conn, meta| Box::pin(async move {
        if meta.idle_for.as_secs() > 60 { conn.ping().await?; }
        Ok(true)  // true = accept, false = reject (close and replace)
    }))
    .after_release(|conn, _meta| Box::pin(async move {
        Ok(true)  // true = return to pool, false = close connection
    }))
    .connect(&db_url).await?;
```

### Multi-Process Pooling

Each binary gets its own `PgPool`. Divide PostgreSQL's `max_connections` across all processes, accounting for the 3-connection superuser reserve.

For a **512MB baseline** PostgreSQL, `max_connections = 20` is appropriate for a single binary (19 for queries + 1 for PgListener). For multi-process deployments, multiply by process count:

```rust
// For a deployment with max_connections = 100 and 3 binaries:
let pool = PgPoolOptions::new()
    .max_connections((100 - 3) / 3)  // ~32 per process
    .connect(&db_url).await?;
```

Adjust PostgreSQL's `max_connections` proportionally when adding binaries. Each PgListener also consumes 1 connection from its pool. Keep the 3-connection superuser reserve for maintenance tasks.

### connect_lazy vs connect

```rust
// Lazy: no connections opened at construction time — good for startup when DB might not be ready
let pool = PgPoolOptions::new().connect_lazy(&db_url)?;

// Eager: opens at least 1 connection immediately, validates config
let pool = PgPoolOptions::new().connect(&db_url).await?;
```

---

## 2. JSONB Storage

### Insert with Json<T>

```rust
use sqlx::types::Json;

sqlx::query!(
    "INSERT INTO search_results (id, data) VALUES ($1, $2)",
    uuid::Uuid::new_v4(),
    Json(&result) as _,  // `as _` casts — tells macro "trust me on the type"
)
.execute(&pool).await?;
```

### Query with Type Annotation

```rust
// Compile-time checked with type annotation
let rows = sqlx::query_as!(
    Row,
    r#"SELECT id, data as "data: Json<SearchResult>", created_at FROM search_results"#
)
.fetch_all(&pool).await?;
```

### JSONB Operators

```rust
// Containment: @>
sqlx::query("SELECT data FROM items WHERE data @> $1")
    .bind(serde_json::json!({"status": "active"}))
    .fetch_all(&pool).await?;

// Path extraction: #>
sqlx::query_scalar::<_, serde_json::Value>("SELECT data #> '{address,city}' FROM users")
    .fetch_one(&pool).await?;

// Key exists: ?
sqlx::query("SELECT data FROM users WHERE data ? 'email'")
    .fetch_all(&pool).await?;
```

### Indexing JSONB

```sql
-- GIN index for containment queries
CREATE INDEX idx_data_gin ON search_results USING GIN(data);

-- Expression index on a specific JSON path
CREATE INDEX idx_data_status ON search_results ((data->>'status'));
```

---

## 3. Full-Text Search

### Schema (tsvector + GIN index)

```sql
CREATE TABLE search_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    fts tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(body, '')), 'B')
    ) STORED,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_fts ON search_results USING GIN(fts);
```

### BM25-like Ranking with ts_rank

```rust
let results = sqlx::query_as::<_, SearchResult>(
    "SELECT *, ts_rank(fts, query) AS rank
     FROM search_results, plainto_tsquery('english', $1) query
     WHERE fts @@ query
     ORDER BY rank DESC
     LIMIT 20"
)
.bind(query_string)
.fetch_all(&pool).await?;
```

### Weight Tuning

```sql
-- Weights: A=1.0, B=0.4, C=0.2, D=0.1
-- title matches 2.5x more important than body matches
CREATE TABLE docs (
    fts tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('english', title), 'A') ||     -- weight 1.0
        setweight(to_tsvector('english', summary), 'B') ||   -- weight 0.4
        setweight(to_tsvector('english', body), 'D')         -- weight 0.1
    ) STORED
);
```

### CJK (Chinese) Text Search

```sql
-- Option 1: zhparser extension (Chinese word segmentation)
CREATE EXTENSION IF NOT EXISTS zhparser;
CREATE TEXT SEARCH CONFIGURATION chinese (PARSER = zhparser);
ALTER TEXT SEARCH CONFIGURATION chinese ADD MAPPING FOR n,v,a,i,e,l WITH simple;

-- Option 2: pg_bigm (2-gram indexing, good for CJK)
CREATE EXTENSION IF NOT EXISTS pg_bigm;
CREATE INDEX idx_title_bigm ON docs USING bigm(title);
```

### Highlighting Results

```sql
SELECT ts_headline('english', body, query, 'MaxWords=50, MinWords=20, StartSel=<mark>, StopSel=</mark>')
FROM search_results, to_tsquery('english', $1) query
WHERE fts @@ query;
```

---

## 4. LISTEN/NOTIFY Live Queries

### PgListener Setup

```rust
use sqlx::postgres::{PgListener, PgNotification};

// Option A: Borrow 1 connection from existing pool (recommended)
let mut listener = PgListener::connect_with(&pool).await?;
listener.listen_all(vec!["search_results", "proxy_status"]).await?;

// Option B: Standalone (creates internal pool of 1)
let mut listener = PgListener::connect("postgresql://localhost/mydb").await?;
```

### Receive Notifications

```rust
// Blocking receive (auto-reconnect on connection loss)
let notification = listener.recv().await?;
// notification.channel() -> &str
// notification.payload() -> &str
// notification.process_id() -> i32

// Non-blocking receive
while let Some(notification) = listener.try_recv().await? {
    process(notification);
}

// Convert to Stream
let mut stream = listener.into_stream();
while let Some(notification) = stream.try_next().await? {
    process(notification);
}
```

### Send Notifications

```rust
// From any connection — propagate the serialize error instead of unwrapping.
// `serde_json::to_string` can fail for non-UTF-8 map keys, NaN/Inf floats,
// and exotic serializer edge cases; silently unwrapping crashes the calling
// request handler with no useful diagnostic.
sqlx::query("SELECT pg_notify($1, $2)")
    .bind("search_results")
    .bind(serde_json::to_string(&payload).map_err(NotifyError::Serialize)?)
    .execute(&pool).await?;

// From a trigger function:
sqlx::query(
    "CREATE OR REPLACE FUNCTION notify_new_result()
     RETURNS TRIGGER AS $$
     BEGIN
         PERFORM pg_notify('search_results', row_to_json(NEW)::text);
         RETURN NEW;
     END;
     $$ LANGUAGE plpgsql"
).execute(&pool).await?;

sqlx::query(
    "CREATE TRIGGER on_new_result
     AFTER INSERT ON search_results
     FOR EACH ROW EXECUTE FUNCTION notify_new_result()"
).execute(&pool).await?;
```

`NotifyError::Serialize` should be a `#[from] serde_json::Error` variant on
your application's error type so the source chain is preserved.

### Production Config

```rust
let mut listener = PgListener::connect_with(&pool).await?;
listener.set_channel_buffer_size(1024);  // buffer for unreceived notifications

// Eager reconnect: immediately reconnect on connection loss
listener.eager_reconnect(true);

// Soft reconnect: wait for recv/recv_timeout to detect loss
listener.eager_reconnect(false);
```

### Budget: PgListener + Pool

PgListener consumes **1 connection** from the pool for its entire lifetime. Budget accordingly:

```rust
let pool = PgPoolOptions::new()
    .max_connections(21)  // 20 for queries + 1 for PgListener
    .connect(&db_url).await?;
```

---

## 5. Migrations

### File Structure

```
migrations/
├── 001_create_search_results.up.sql
├── 001_create_search_results.down.sql
├── 002_add_fts_index.up.sql
├── 002_add_fts_index.down.sql
├── 003_create_proxy_status.sql  # simple (non-reversible)
```

### Naming Convention

```rust
// sqlx-core/src/migrate/source.rs:196-223
// VERSION must parse as i64, > 0
// Reversible: <VERSION>_<DESCRIPTION>.up.sql + .down.sql
// Simple: <VERSION>_<DESCRIPTION>.sql
```

### Migration SQL Features

```sql
-- no-transaction (for CREATE INDEX CONCURRENTLY)
-- add this as first line of the migration file
CREATE INDEX CONCURRENTLY idx_fts ON search_results USING GIN(fts);
```

### Running Migrations

```rust
// Compile-time embedding (recommended)
let migrator = sqlx::migrate!("./migrations");
migrator.run(&pool).await?;

// Runtime loading
let migrator = sqlx::migrate::Migrator::new(Path::new("./migrations")).await?;
migrator.run(&pool).await?;

// To specific version
migrator.run_to(&pool, 5).await?;

// Revert
migrator.undo(&pool, 3).await?;
```

### CLI Commands

```bash
sqlx migrate add <description>     # create new migration file
sqlx migrate run                   # apply pending migrations
sqlx migrate revert                # revert last migration
sqlx migrate info                  # show applied migrations
```

---

## 6. Compile-Time Query Checking

### query! Macro

```rust
// query!() infers return type from SQL schema
let user = sqlx::query!("SELECT id, name FROM users WHERE id = $1", id)
    .fetch_one(&pool).await?;
// user.id, user.name available with correct types
```

### query_as! Macro

```rust
#[derive(sqlx::FromRow)]
struct User { id: i64, name: String }

let user = sqlx::query_as!(User, "SELECT id, name FROM users WHERE id = $1", id)
    .fetch_one(&pool).await?;
```

### Offline Mode

```bash
# Generate/update .sqlx cache
SQLX_OFFLINE=true cargo sqlx prepare

# Build without database
SQLX_OFFLINE=true cargo build
```

### JSON Type Annotations

For JSONB fields, use explicit type annotations:

```rust
// In query_as! with JSONB column:
sqlx::query_as!(Row, r#"SELECT data as "data: Json<SearchResult>" FROM results"#)

// In query! with JSONB cast:
sqlx::query!("INSERT INTO results (data) VALUES ($1)", Json(&data) as _)
```

### sqlx.toml Config

```toml
[sqlx]
offline = true              # Force offline mode
offline_dir = ".sqlx"       # Custom cache directory
```

### Cache Files

Each query creates `.sqlx/query-<sha256>.json`. These must be committed to version control. Checksums are SHA-384 of SQL content. Changing a previously applied migration's content causes `MigrateError::VersionMismatch`.

---

## 7. Error Handling

### Error Structure

```rust
use sqlx::Error;

match err {
    Error::RowNotFound => { /* fetch_one with no results */ }
    Error::PoolTimedOut => { /* acquire() timeout */ }
    Error::PoolClosed => { /* pool shut down during acquire */ }
    _ => {}
}
```

### Downcast to PostgreSQL Error

```rust
use sqlx::postgres::PgDatabaseError;

if let Some(pg_err) = err
    .as_database_error()
    .and_then(|e| e.try_downcast_ref::<PgDatabaseError>())
{
    eprintln!("SQLSTATE: {}", pg_err.code());         // e.g. "23505"
    eprintln!("Detail: {:?}", pg_err.detail());
    eprintln!("Constraint: {:?}", pg_err.constraint());
    eprintln!("Table: {:?}", pg_err.table());
    eprintln!("Column: {:?}", pg_err.column());
}
```

### ErrorKind (Programmatic Handling)

```rust
match err.kind() {
    ErrorKind::UniqueViolation => { /* conflict */ }
    ErrorKind::ForeignKeyViolation => { /* FK broken */ }
    ErrorKind::NotNullViolation => { /* null in NOT NULL */ }
    ErrorKind::CheckViolation => { /* check constraint failed */ }
    _ => {}
}
```

### Pool Error Handling

```rust
let conn = match pool.acquire().await {
    Ok(conn) => conn,
    Err(Error::PoolTimedOut) => {
        tracing::error!("pool exhausted, consider increasing max_connections");
        return Err(AppError::Busy);
    }
    Err(e) => return Err(e.into()),
};
```

---

## 8. Transactions

### Basic Transaction

```rust
let mut tx = pool.begin().await?;
sqlx::query("INSERT INTO results (id, data) VALUES ($1, $2)")
    .bind(id).bind(Json(&data))
    .execute(&mut *tx).await?;
tx.commit().await?;
// Drop without commit = implicit rollback
```

### Nested Transactions (Savepoints)

```rust
let mut tx = pool.begin().await?;
// ... outer work ...
let mut inner = tx.begin().await?;  // creates SAVEPOINT
// ... inner work ...
inner.commit().await?;              // releases SAVEPOINT
tx.commit().await?;                 // COMMIT
```

### Custom Isolation Level

```rust
let mut tx = pool.begin_with("BEGIN ISOLATION LEVEL SERIALIZABLE").await?;
// ... critical section ...
tx.commit().await?;
```

### Advisory Locks

```rust
use sqlx::postgres::PgAdvisoryLock;

// Session-scoped lock (survives transactions)
let lock = PgAdvisoryLock::new("migration-lock");
let _guard = lock.acquire(&mut *conn).await?;  // blocks until acquired

// Transaction-scoped lock (released on commit/rollback)
sqlx::query("SELECT pg_advisory_xact_lock($1)")
    .bind(12345_i64)
    .execute(&mut *tx).await?;
```

---

## 9. Performance Tuning (512MB Config)

### postgresql.conf for 512MB RAM

```ini
shared_buffers = 64MB              # 12.5% of RAM
effective_cache_size = 128MB       # OS cache hint
work_mem = 2MB                     # per-operation limit (keep low)
maintenance_work_mem = 32MB        # for VACUUM, CREATE INDEX
max_connections = 20               # 512MB baseline (19 queries + 1 listener); multiply by N processes for multi-process
synchronous_commit = off           # speed tradeoff (acceptable for proxy data)
wal_compression = on               # reduce WAL size
autovacuum_max_workers = 1         # lightweight
random_page_cost = 1.1             # SSD tuned
effective_io_concurrency = 200     # SSD concurrency
```

### sqlx Pool Tuning

```rust
let pool = PgPoolOptions::new()
    .max_connections(10)           // conservative, leave headroom
    .min_connections(1)            // pre-warm one connection
    .acquire_timeout(Duration::from_secs(3))
    .idle_timeout(Duration::from_secs(300))  // 5 min
    .max_lifetime(Duration::from_secs(1800)) // 30 min
    .test_before_acquire(true)
    .connect(&db_url).await?;
```

### When to tune up

- Increase `work_mem` (up to 16MB) if you run large sorts or hash joins
- Increase `shared_buffers` (up to 128MB) if you have spare RAM
- Increase `max_connections` if you have >2 binaries connecting
- Consider connection pooling (PgBouncer) if you have many short-lived connections

---

## 10. Multi-Process Access

### Architecture

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  searxrs2    │  │  proxytest   │  │  warpproxy   │
│  PgPool (10) │  │  PgPool (5)  │  │  PgPool (5)  │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │
       └─────────────────┼─────────────────┘
                         │
               ┌─────────▼─────────┐
               │   PostgreSQL 17    │
               │ max_connections=20 │
               └───────────────────┘
```

- Each binary has its own `PgPool`
- Total connections across all pools must fit within PostgreSQL's `max_connections`
- PgListener in each binary consumes 1 connection from its pool
- SQL queries use the pool normally
- Write contention is handled by PostgreSQL's MVCC (no application-level locking needed)

### Read-Mostly Pattern

For search proxy workloads (lots of reads, occasional writes):

```rust
// Reader pool (most connections)
let read_pool = PgPoolOptions::new().max_connections(15).connect(&db_url).await?;

// Writer pool (fewer connections, transactions)
let write_pool = PgPoolOptions::new().max_connections(3).connect(&db_url).await?;

// Listener (dedicated)
let mut listener = PgListener::connect_with(&read_pool).await?;
```

---

## 11. TTL (Time-To-Live)

### pg_cron Extension

```sql
-- Install (once per DB, requires superuser to install extension)
CREATE EXTENSION IF NOT EXISTS pg_cron;

-- Schedule cleanup every hour
SELECT cron.schedule(
    'cleanup-search-results',
    '0 * * * *',
    $$DELETE FROM search_results WHERE created_at < now() - INTERVAL '30 days'$$
);

-- Schedule cleanup of raw HTML storage daily
SELECT cron.schedule(
    'cleanup-raw-html',
    '0 3 * * *',  -- 3 AM daily
    $$DELETE FROM raw_html_storage WHERE created_at < now() - INTERVAL '30 days'$$
);

-- List scheduled jobs
SELECT * FROM cron.job;

-- Unschedule
SELECT cron.unschedule('cleanup-search-results');
```

### Partition-Based TTL (High-Volume Alternative)

```sql
-- Create partitioned table
CREATE TABLE request_logs (
    id BIGSERIAL,
    data JSONB,
    created_at TIMESTAMPTZ NOT NULL
) PARTITION BY RANGE (created_at);

-- Day partitions
CREATE TABLE request_logs_20260601 PARTITION OF request_logs
    FOR VALUES FROM ('2026-06-01') TO ('2026-06-02');

-- Automatic TTL: drop old partitions
DROP TABLE request_logs_20260501;  -- 30 days old
```

This is more efficient than `DELETE` for high-volume write workloads.

---

## 12. Pitfalls

1. **PgListener connection leak**: PgListener holds a connection until dropped. For shared pool mode, budget 1 connection for the listener.
2. **Notification buffer overflow**: PostgreSQL drops notifications when the client-side buffer is full. Set reasonable buffer sizes and handle `recv()` returning `None`.
3. **Notifications during disconnect**: Notifications received while the listener is reconnecting are silently lost. Use `eager_reconnect(false)` + manual recovery logic for critical notifications.
4. **Advisory locks are per-connection**: Don't acquire on a pooled connection expecting it to persist. Use transaction-scoped locks (`pg_advisory_xact_lock`) for transactional safety.
5. **`test_before_acquire(true)` overhead**: Each pool acquire does a ping roundtrip. For latency-sensitive paths, set `false` and use `before_acquire` for smarter health checks.
6. **sqlx offline cache drift**: If you change a migration SQL, the cached query descriptions become stale. Always run `cargo sqlx prepare` after migration changes.
7. **max_connections ceiling**: Each `PgPool` + `PgListener` consumes connections. Total across all processes must fit within PostgreSQL's `max_connections` minus 3 (superuser reserve).
