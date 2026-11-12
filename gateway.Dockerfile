# Pinned by digest for reproducible builds.
#
# Multi-stage build with cargo-chef for reproducible dependency caching
# across the whole workspace.
FROM rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS chef
# Keep the reproducible Bookworm base pinned by digest, then install the
# repository's current stable compiler explicitly so Docker image publication
# lag cannot leave production builds on an older Rust release.
RUN rustup toolchain install 1.99.0 --profile minimal
ENV RUSTUP_TOOLCHAIN=1.99.0
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# The workspace patches crates.io dependencies to local sources. Those path
# dependencies must exist while cargo-chef cooks the dependency layer.
COPY third-party /build/third-party
# Match the final package selection exactly so Cargo can reuse the cooked
# library + binary dependency graph after the real sources are copied in.
RUN cargo chef cook --recipe-path recipe.json --locked --release \
      --package gateway-example
COPY . .
RUN cargo build --locked --release -p gateway-example

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
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