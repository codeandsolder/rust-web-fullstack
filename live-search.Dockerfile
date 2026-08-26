# Pinned by digest for reproducible builds.
#
# Multi-stage build:
#   1. chef: install cargo-chef once, reused by planner + builder.
#   2. planner: produce a recipe.json over the whole workspace.
#   3. builder: cargo chef cook the recipe (cached dependency build),
#      then copy source and build the binary.
FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS chef
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN rustup target add wasm32-unknown-unknown && \
    cargo install wasm-bindgen-cli --version 0.2.126 --locked && \
    cargo install stylance-cli --locked && \
    cargo install leptosfmt --locked && \
    cargo install sccache --locked
ENV RUSTC_WRAPPER=sccache
COPY --from=planner /build/recipe.json recipe.json
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo chef cook --recipe-path recipe.json --locked --release \
      --target wasm32-unknown-unknown --features live-search/hydrate && \
    cargo chef cook --recipe-path recipe.json --locked --release \
      --features live-search/ssr
COPY . .
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo build --locked --release -p live-search --lib \
      --target wasm32-unknown-unknown --features hydrate && \
    mkdir -p /build/site/pkg && \
    wasm-bindgen \
      --target web \
      --out-dir /build/site/pkg \
      --out-name live_search \
      /build/target/wasm32-unknown-unknown/release/live_search.wasm && \
    stylance build
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo build --locked --release -p live-search --features ssr

FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df
RUN groupadd -r app && \
    useradd -r -g app -d /app -s /usr/sbin/nologin app && \
    mkdir -p /app && chown -R app:app /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates wget && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/live-search /app/
COPY --from=builder /build/site/pkg /app/site/pkg
# SQLx migrations are embedded at compile time from the workspace-level
# `migrations/` directory; no runtime migration files are required.
RUN mkdir -p /app/pkg && cp /app/site/pkg/live-search.css /app/pkg/ 2>/dev/null || true && \
    cp /app/site/pkg/live_search.js /app/pkg/ 2>/dev/null || true && \
    cp /app/site/pkg/live_search_bg.wasm /app/pkg/ 2>/dev/null || true
ENV LEPTOS_OUTPUT_NAME=live_search
ENV LEPTOS_SITE_PKG_DIR=/app/pkg
USER app
EXPOSE 3000
CMD ["/app/live-search"]
