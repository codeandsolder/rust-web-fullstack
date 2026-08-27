# Pinned by digest for reproducible builds.
#
# Multi-stage build with cargo-chef for reproducible dependency caching
# across the whole workspace.
FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS chef
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN cargo install sccache --locked && \
    cargo install leptosfmt --locked
ENV RUSTC_WRAPPER=sccache
COPY --from=planner /build/recipe.json recipe.json
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo chef cook --recipe-path recipe.json --locked --release \
      --bin gateway-example
COPY . .
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo build --locked --release -p gateway-example

FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df
RUN groupadd -r app && \
    useradd -r -g app -d /app -s /usr/sbin/nologin app && \
    mkdir -p /app && chown -R app:app /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates wget && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/gateway-example /app/
# SQLx migrations are embedded at compile time from the workspace-level
# `migrations/` directory; the runtime image does not need SQL files.
USER app
EXPOSE 3001
# Production: pass JWT_PRIVATE_KEY_PEM / JWT_PUBLIC_KEY_PEM / ADMIN_PASSWORD /
# ADMIN_USER_ID / DATABASE_URL via the deployment platform. Use --dev-keys only
# with ALLOW_DEV_KEYS=1.
CMD ["/app/gateway-example"]
