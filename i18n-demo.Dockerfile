# Pinned by digest for reproducible builds.
FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS builder
WORKDIR /build
RUN rustup target add wasm32-unknown-unknown && \
    cargo install wasm-bindgen-cli --version 0.2.126 --locked

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY gateway/Cargo.toml gateway/
COPY live-search/Cargo.toml live-search/
COPY i18n-demo/Cargo.toml i18n-demo/
COPY e2e-tests/Cargo.toml e2e-tests/
RUN mkdir -p gateway/src live-search/src i18n-demo/src e2e-tests/src && \
    echo "// dummy" > gateway/src/lib.rs && \
    echo "// dummy" > live-search/src/lib.rs && \
    echo "// dummy" > i18n-demo/src/lib.rs && \
    echo "// dummy" > e2e-tests/src/lib.rs && \
    echo "// dummy" > i18n-demo/build.rs && \
    cargo fetch

# Build i18n-demo only
COPY . .
RUN cargo build --locked --release -p i18n-demo --lib --target wasm32-unknown-unknown --features hydrate && \
    mkdir -p /build/pkg && \
    wasm-bindgen \
      --target web \
      --out-dir /build/pkg \
      --out-name i18n_demo \
      /build/target/wasm32-unknown-unknown/release/i18n_demo.wasm && \
    touch /build/pkg/i18n-demo.css
RUN cargo build --locked --release -p i18n-demo --features ssr

# Pinned by digest for reproducible builds.
FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df
RUN groupadd -r app && useradd -r -g app -d /app -s /usr/sbin/nologin app && chown -R app:app /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/i18n-demo /app/
COPY --from=builder /build/pkg /app/pkg
EXPOSE 3002
USER app
# LEPTOS_OUTPUT_NAME must match the wasm-bindgen `--out-name` used at build
# time (i18n_demo, with underscore). Without this, the SSR shell emits
# empty hydration paths (`///pkg///.js`) and the browser 404s on the JS
# module — the page renders but JavaScript never runs.
ENV LEPTOS_OUTPUT_NAME=i18n_demo
CMD ["/app/i18n-demo"]
