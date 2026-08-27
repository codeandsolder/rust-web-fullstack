# Pinned by digest for reproducible builds.
#
# Multi-stage build for the i18n-demo Leptos crate.
FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS chef
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN rustup target add wasm32-unknown-unknown && \
    cargo install wasm-bindgen-cli --version 0.2.126 --locked && \
    cargo install sccache --locked && \
    cargo install leptosfmt --locked
ENV RUSTC_WRAPPER=sccache
COPY --from=planner /build/recipe.json recipe.json
# Keep wasm/native dependency cooks scoped to this crate. A workspace-wide wasm
# cook enables native networking dependencies from unrelated members.
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo chef cook --recipe-path recipe.json --locked --release \
      --package i18n-demo --target wasm32-unknown-unknown --no-default-features --features hydrate && \
    cargo chef cook --recipe-path recipe.json --locked --release \
      --package i18n-demo --no-default-features --features ssr
COPY . .
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo build --locked --release -p i18n-demo --lib \
      --target wasm32-unknown-unknown --features hydrate && \
    mkdir -p /build/site/pkg && \
    wasm-bindgen \
      --target web \
      --out-dir /build/site/pkg \
      --out-name i18n_demo \
      /build/target/wasm32-unknown-unknown/release/i18n_demo.wasm
RUN --mount=type=cache,target=/var/cache/sccache \
    cargo build --locked --release -p i18n-demo --features ssr

FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df
RUN groupadd -r app && \
    useradd -r -g app -d /app -s /usr/sbin/nologin app && \
    mkdir -p /app && chown -R app:app /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates wget && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/i18n-demo /app/
COPY --from=builder /build/site/pkg /app/site/pkg
# i18n-demo uses handwritten CSS (no Stylance); copy it alongside.
COPY i18n-demo/src/styles /app/site/pkg/styles
RUN mkdir -p /app/pkg && cp -r /app/site/pkg/* /app/pkg/
ENV LEPTOS_OUTPUT_NAME=i18n_demo
ENV LEPTOS_SITE_PKG_DIR=/app/pkg
USER app
EXPOSE 3002
CMD ["/app/i18n-demo"]
