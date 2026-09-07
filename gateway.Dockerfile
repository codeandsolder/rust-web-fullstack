# Pinned by digest for reproducible builds.
#
# Multi-stage build with cargo-chef for reproducible dependency caching
# across the whole workspace.
FROM rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922 AS chef
# The official 1.98 image carries 1.98.0; install the patched stable compiler
# explicitly so production builds do not stay on a known superseded point release.
RUN rustup toolchain install 1.98.1 --profile minimal
ENV RUSTUP_TOOLCHAIN=1.98.1
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# Match the final package selection exactly so Cargo can reuse the cooked
# library + binary dependency graph after the real sources are copied in.
RUN cargo chef cook --recipe-path recipe.json --locked --release \
      --package gateway-example
COPY . .
RUN cargo build --locked --release -p gateway-example

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