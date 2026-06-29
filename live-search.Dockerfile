# TODO: pin by digest — e.g. rust:1.94-bookworm@sha256:...
FROM rust:1.94-bookworm AS builder
WORKDIR /build
RUN rustup target add wasm32-unknown-unknown && \
    cargo install wasm-bindgen-cli --version 0.2.126 --locked

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY gateway/Cargo.toml gateway/
COPY live-search/Cargo.toml live-search/
COPY e2e-tests/Cargo.toml e2e-tests/
RUN mkdir -p gateway/src live-search/src e2e-tests/src && \
    echo "// dummy" > gateway/src/lib.rs && \
    echo "// dummy" > live-search/src/lib.rs && \
    echo "// dummy" > e2e-tests/src/lib.rs && \
    cargo fetch

# Build live-search only
COPY . .
RUN cargo build --locked --release -p live-search --lib --target wasm32-unknown-unknown --features hydrate && \
    mkdir -p /build/pkg && \
    wasm-bindgen \
      --target web \
      --out-dir /build/pkg \
      --out-name live_search \
      /build/target/wasm32-unknown-unknown/release/live_search.wasm && \
    touch /build/pkg/live-search.css
RUN cargo build --locked --release -p live-search --features ssr

# TODO: pin by digest — e.g. debian:bookworm-slim@sha256:...
FROM debian:bookworm-slim
RUN groupadd -r app && useradd -r -g app -d /app -s /usr/sbin/nologin app && chown -R app:app /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/live-search /app/
COPY --from=builder /build/pkg /app/pkg
COPY live-search/migrations /app/migrations
EXPOSE 3000
USER app
CMD ["/app/live-search"]
