# PostgreSQL/sqlx Patterns Reference

Companion to [`SKILL.md`](../SKILL.md). The canonical implementation is in
`live-search/src/db.rs` and root `migrations/`.

## Pool sizing

A `PgListener` uses a database connection in addition to normal request traffic.
Budget for it explicitly when choosing pool and PostgreSQL connection limits.

Validate pool configuration at startup:

```text
max_connections > 0
min_connections <= max_connections
acquire_timeout > 0
idle_timeout > 0
max_lifetime > 0
```

A configuration object that can describe impossible pool bounds is not truly
typed configuration.

## One migration history per database

SQLx tracks applied migration versions in `_sqlx_migrations`. Two services that
share a database but each resolve a disjoint migration directory will treat the
other directory's applied versions as missing.

This repository therefore has one root migration history:

```text
migrations/
  001 ... search_results
  002 ... URL uniqueness
  003 ... pg_trgm index
  004 ... durable event sequence
  100 ... refresh_tokens
  101 ... unique refresh-token digest
```

Both production binaries and testcontainers resolve that same history.

## Search schema

`search_results` supports both PostgreSQL full-text search and a trigram title
fallback. The application query uses both mechanisms, so the pg_trgm index is
not unused schema.

A representative shape is:

```sql
WITH q AS (SELECT plainto_tsquery('english', $1) AS tsq)
SELECT id, title, url, snippet, created_at
FROM search_results, q
WHERE fts @@ q.tsq OR title % $1
ORDER BY
  (fts @@ q.tsq) DESC,
  GREATEST(ts_rank_cd(fts, q.tsq), similarity(title, $1)) DESC,
  created_at DESC,
  id DESC
LIMIT 20;
```

`ts_rank`/`ts_rank_cd` are PostgreSQL full-text relevance functions; do not call
them BM25. If BM25 behavior is required, implement/use a system that actually
provides it.

## LISTEN/NOTIFY semantics

NOTIFY is low-latency signaling, not a durable event log. A disconnected
listener can miss notifications. The application must be able to reconstruct
missed work from durable database state.

### Why UUIDv4 is not a reconnect cursor

Random UUID order has no relationship to insert/commit chronology:

```sql
WHERE id > $last_uuid
```

can permanently skip rows inserted during a disconnect.

### Why plain BIGSERIAL can also be wrong

Sequence values are allocated before transaction commit. Concurrent transactions
can allocate 10 then 11 but commit 11 before 10. If a consumer observes 11 and
advances a high-water mark to 11, the later-visible row 10 can be skipped.

### Canonical event sequence

Migration 004 assigns `event_seq` under a transaction-scoped PostgreSQL advisory
lock. The lock is held to COMMIT/ROLLBACK, so visible event order and sequence
order agree.

Reconnect logic then safely does:

```sql
SELECT ...
FROM search_results
WHERE event_seq > $1
ORDER BY event_seq ASC
LIMIT $2;
```

and pages until fewer than a full batch are returned.

This serialized allocator is appropriate for the demo's modest write workload.
For high write rates, move durable events into a dedicated outbox/log design
rather than removing commit-order safety.

## Notification payloads

Keep NOTIFY payloads small. The canonical trigger sends enough identity data to
wake/fetch the row; application code then loads the authoritative row from the
database.

If a notification references a row that has disappeared, distinguish that case
from a database fetch error. Do not collapse `.await` errors into `None` with a
blanket `.ok().flatten()` because it hides operational failures.

## Reconnect loop

A robust listener loop:

1. connects and LISTENs,
2. performs durable replay from the last sequence,
3. consumes live notifications,
4. suppresses notifications already covered by replay,
5. backs off on failure,
6. races reconnect sleep and `recv()` against a cancellation token.

The cursor must advance during replay itself, not only after the next live
notification, or consecutive reconnects replay the same rows repeatedly.

## Cache invalidation

Search cache invalidation is tied to the same change/event contract. If writes
expand from INSERT-only to UPDATE/DELETE, update the trigger/event semantics and
cache invalidation together.

A cache TTL is a fallback, not a substitute for correct invalidation when the
application promises live updates.

## Cursor pagination

For user-facing result pagination, a stable keyset cursor such as
`(created_at, id)` is appropriate when the sort is:

```sql
ORDER BY created_at DESC, id DESC
```

The page predicate mirrors that tuple ordering:

```sql
AND (created_at, id) < ($cursor_time, $cursor_id)
```

This cursor serves a different purpose from the durable `event_seq` reconnect
cursor. Do not conflate API pagination order with event delivery order.

## Refresh-token storage

Refresh tokens store:

- `jti`,
- token family id,
- subject UUID,
- **hash** of the opaque raw token,
- expiry/revocation timestamps,
- `replaced_by` lineage.

The digest has a UNIQUE index. Rotation locks the active row in a transaction,
revokes it, records its replacement, inserts the replacement, and commits
atomically.

A lookup of an already-used token can still identify its family and revoke all
active members as replay response.

Expired/revoked-token retention is an operational policy. A long-running real
service should define cleanup rather than allow the table to grow forever.

## Testcontainers

Fresh test databases run the exact root migration history. This catches migration
compatibility failures that hand-created schemas or raw DDL fixtures can hide.

Tests should seed only the rows needed for their assertions and must not rely on
cross-test ordering.
