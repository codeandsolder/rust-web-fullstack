# Custom Woodpecker CI image with leptosfmt preinstalled.
#
# Build:  docker build -t woodpecker-rust-leptosfmt:latest -f ci/leptosfmt.Dockerfile .
# Push:   docker push woodpecker-rust-leptosfmt:latest
# Update: bump the pin hash and re-push.
#
# Uses the same Rust digest as the other CI steps for reproducibility.
FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS build

# Install leptosfmt into a global cargo bin dir so the runtime layer can COPY it.
RUN cargo install leptosfmt --locked

# Slim runtime: same digest, only the cargo bin dir copied.
FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55
COPY --from=build /usr/local/cargo/bin/leptosfmt /usr/local/bin/leptosfmt
