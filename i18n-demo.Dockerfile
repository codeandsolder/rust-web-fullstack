# Pinned by digest for reproducible builds.
#
# Multi-stage build for the i18n-demo Leptos crate.
FROM rust:1.100.0-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS chef
# The digest-pinned image bootstraps rustup; the native manifest selects the
# compiler for cargo-chef, dependency cooking and final production builds.
WORKDIR /build
COPY rust-toolchain.toml ./
RUN rustup show && cargo install cargo-chef --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY Cargo.lock ./
COPY scripts/wasm-bindgen-version.sh scripts/wasm-bindgen-version.sh
RUN rustup target add wasm32-unknown-unknown && \
    cargo install wasm-bindgen-cli --version "=$(sh scripts/wasm-bindgen-version.sh)" --locked
COPY --from=planner /build/recipe.json recipe.json
# The workspace patches crates.io dependencies to local sources. Those path
# dependencies must exist while cargo-chef cooks the dependency layer.
COPY third-party /build/third-party
# Keep wasm/native dependency cooks scoped to this crate. A workspace-wide wasm
# cook enables native networking dependencies from unrelated members.
RUN cargo chef cook --recipe-path recipe.json --locked --release \
      --package i18n-demo --target wasm32-unknown-unknown --no-default-features --features hydrate && \
    cargo chef cook --recipe-path recipe.json --locked --release \
      --package i18n-demo --no-default-features --features ssr
COPY . .
RUN cargo build --locked --release -p i18n-demo --lib \
      --target wasm32-unknown-unknown --features hydrate && \
    mkdir -p /build/site/pkg && \
    wasm-bindgen \
      --target web \
      --out-dir /build/site/pkg \
      --out-name i18n_demo \
      /build/target/wasm32-unknown-unknown/release/i18n_demo.wasm
RUN cargo build --locked --release -p i18n-demo --features ssr

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
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