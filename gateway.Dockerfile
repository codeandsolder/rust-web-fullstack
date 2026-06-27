FROM rust:1.94-bookworm AS builder
WORKDIR /build

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

# Build gateway only
COPY . .
RUN cargo build --release -p gateway-example

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/gateway-example /app/
EXPOSE 3001
CMD ["/app/gateway-example"]
