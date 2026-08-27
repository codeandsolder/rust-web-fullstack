# Architecture Patterns Reference

This workspace is a reference implementation, not a framework. Prefer small
boundaries that make failure ownership obvious.

## Workspace boundaries

- `crates/domain`: pure domain values and invariants; no Axum/Leptos/sqlx.
- `crates/config`: typed non-secret runtime configuration and validation.
- `live-search`: one full-stack Leptos application.
- `gateway`: one Axum service gateway/auth example.
- `i18n-demo`: one independent Leptos example.
- `e2e-tests`: black-box/in-process integration consumers.

Shared code belongs in a shared crate only when multiple applications actually
need the same abstraction. Do not create a “common” crate solely to make the
workspace look layered.

## Domain invariants

A newtype exists to make invalid states harder to express. Every public
constructor/deserializer must therefore preserve the same invariant.

`UserId` rejects `Uuid::nil()`. Exposing a public tuple field or unchecked `new`
would defeat the reason the type exists.

## Configuration ownership

Typed configuration is loaded once at process startup and validated before
expensive resources are created.

`crates/config` owns non-secret runtime settings such as ports, pool sizes, CORS
origins, session cookie security, SSE capacity, and token TTLs. Environment
variables override nested fields with `RWF_*` + `__` separators.

Secrets and deployment credentials remain explicit environment values.

Do not load the same setting independently in two configuration systems and then
pass both copies through state. That creates split-brain behavior where logs,
state metadata, and actual security decisions disagree.

## Database ownership

The Compose topology uses one PostgreSQL database for live-search and gateway.
Therefore the workspace also uses **one** root SQLx migration history.

Rules:

1. every process that migrates that database resolves the same migration set,
2. testcontainers use the same history,
3. no test-only raw DDL bypasses SQLx migration bookkeeping,
4. service-local migration directories require a separate database/schema and
   explicit ownership model.

## Startup sequence

A production service should fail before accepting traffic if a dependency is
required for its core contract.

Gateway startup:

```text
load + validate config
→ load keys/secrets
→ create required PostgreSQL pool
→ run shared migrations
→ build service modules/router
→ bind listener
→ serve
```

Because login always issues a refresh token, the gateway database is required;
starting without it only to make `/auth/login` return 503 is not a useful degraded
mode.

Live-search follows the same principle for its database, migrations, cache,
broadcast channel, and listener task.

## Structured concurrency

Own long-running tasks. The live-search server returns a handle containing:

- a `CancellationToken`,
- a `JoinSet` of owned tasks,
- the database pool.

Blocking waits/reconnect backoff race cancellation with `tokio::select!`.

Shutdown order:

```text
signal cancellation
→ allow HTTP/PgListener tasks to finish
→ drain JoinSet and observe failures
→ close PgPool
→ force-flush/shutdown telemetry
```

Do not close the pool before tasks that still need it have drained.

A binary with no independently spawned background work does not need to invent a
`JoinSet` merely for symmetry; use the lifecycle primitives demanded by its
actual ownership graph.

## Liveness and readiness

Liveness answers “is the process responsive?” Readiness answers “can this
instance serve its required contract?”

- live-search `/health`: liveness,
- live-search `/readyz`: PostgreSQL readiness,
- gateway `/health`: aggregate service-module + required database readiness.

`ServiceModule::health_check` has no unconditional default. Adding a module means
choosing and implementing its health semantics.

## Event architecture

LISTEN/NOTIFY is not the durability layer. For live-search:

```text
search_results INSERT
→ commit-ordered durable event_seq
→ PostgreSQL NOTIFY wakeup
→ PgListener fetch/replay
→ Tokio broadcast
→ Axum SSE
→ browser named EventSource subscriptions
```

Reconnect recovery is based on durable database state, not on notification
history or UUID ordering.

The current event sequence uses a transaction-scoped advisory lock so sequence
order matches commit visibility. That intentionally serializes inserts for this
demo. A higher-throughput architecture should use a dedicated outbox/event table
or another durable log design rather than casually removing the ordering
property.

## Cache architecture

The Moka search cache is an optimization, never a source of truth. Database/event
paths invalidate it when search-result changes are delivered/replayed.

If UPDATE/DELETE semantics are added to the table, the database trigger/event
contract and cache invalidation contract must be expanded together. Do not add a
new mutating path that bypasses invalidation and rely on TTL to hide it.

## Gateway modules

`ServiceModule` provides metadata, a nested router, enablement, and an explicit
health probe. Modules share `GatewayState` but should not reach through it for
unrelated implementation details.

Public handlers should accept typed extractors/DTOs. A `HashMap<String, String>`
query bag is not the canonical example when the endpoint has a known schema.

## Authentication boundaries

The gateway demonstrates two credential families:

- Bearer JWT + rotating refresh token,
- cookie session + synchronizer CSRF.

They coexist to demonstrate patterns, not because every production login flow
should combine them.

The password demo has one configured identity. Production identity/auth belongs
behind a real user store.

Refresh-token lineage is durable DB state. Access JWTs are stateless and remain
valid until expiry unless an application adds explicit access-token revocation.

## Reverse proxies

Peer-IP rate limiting is safe for direct connections but collapses clients behind
a proxy. Forwarded-IP extraction solves that only after a trusted proxy boundary
is configured to remove spoofed forwarding headers. Treat proxy trust as a
deployment architecture decision, not a one-line extractor swap.

## Observability

Trace export and context propagation are separate steps:

1. initialize an OTLP exporter/tracer provider,
2. install W3C propagator,
3. install Axum middleware that extracts incoming HTTP trace context,
4. put request tracing middleware on all intended routes,
5. flush/shutdown the provider on process exit.

OTLP/HTTP defaults to port 4318 in these examples.

Metrics are independent from trace export even when a demo uses one feature flag
to expose both.

## Deployment images

Runtime images:

- use pinned base-image digests,
- run as a non-root user,
- explicitly create/chown `/app`,
- copy only required runtime artifacts.

Do not depend on distro-specific `useradd` behavior to create an application
home directory.

BuildKit cache mounts require a BuildKit-capable Docker build environment. CI
must provide an actual Docker daemon/socket/service for `docker build` and for
testcontainers.

## Documentation architecture

Large duplicated cookbooks age badly. `SKILL.md` is the maintained condensed
contract; files under `references/` provide focused elaboration. When a behavior
changes, update code, tests, and the specific reference text in the same PR.
