# Custom Woodpecker CI image with leptosfmt preinstalled.
#
# Build:  docker build -t woodpecker-rust-leptosfmt:latest -f ci/leptosfmt.Dockerfile .
# Push:   docker push woodpecker-rust-leptosfmt:latest
# Update: bump the pin hash and re-push.
#
# Uses the same Rust digest as the other CI steps for reproducibility.
FROM rust:1.100-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS build

# Install leptosfmt into a global cargo bin dir so the runtime layer can COPY it.
RUN rustup toolchain install 1.99.0 --profile minimal && \
    RUSTUP_TOOLCHAIN=1.99.0 cargo install leptosfmt --locked

# Slim runtime: same digest, only the cargo bin dir copied.
FROM rust:1.100-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea
COPY --from=build /usr/local/cargo/bin/leptosfmt /usr/local/bin/leptosfmt
