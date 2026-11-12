# Custom Woodpecker CI image with leptosfmt preinstalled.
#
# Build:  docker build -t woodpecker-rust-leptosfmt:latest -f ci/leptosfmt.Dockerfile .
# Push:   docker push woodpecker-rust-leptosfmt:latest
# Update: bump the pin hash and re-push.
#
# Uses the same Rust digest as the other CI steps for reproducibility.
FROM rust:1.99-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS build

# Install leptosfmt into a global cargo bin dir so the runtime layer can COPY it.
RUN rustup toolchain install 1.99.0 --profile minimal && \
    RUSTUP_TOOLCHAIN=1.99.0 cargo install leptosfmt --locked

# Slim runtime: same digest, only the cargo bin dir copied.
FROM rust:1.99-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c
COPY --from=build /usr/local/cargo/bin/leptosfmt /usr/local/bin/leptosfmt
