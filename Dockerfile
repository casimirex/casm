# CASIMIR container image.
#
# Two stages: a builder with the full Rust toolchain, and a distroless runtime carrying
# nothing but the two binaries and the libraries they link against.
#
# Distroless rather than Alpine: CASIMIR needs no shell, no package manager, and no libc
# variant of its own choosing. What it does need is for the image to have as little in it
# as possible, because every binary in a container is a binary somebody has to patch.
#
#   docker build -t casm .
#   docker run --rm -v "$PWD:/work" casm validate /work/architecture.yaml

FROM rust:1.90-bookworm AS builder

WORKDIR /build

# Manifests first, so a source-only change does not invalidate the dependency layer.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# `--locked` is deliberate: a container build that silently resolved a different
# dependency graph than CI would defeat the point of pinning one.
RUN cargo build --release --locked --bin casm --bin casm-lsp \
    && strip target/release/casm target/release/casm-lsp

# `cc` carries the glibc the binaries link against; `static` would carry none at all, but
# would require a musl target and a different build.
FROM gcr.io/distroless/cc-debian12:nonroot

LABEL org.opencontainers.image.title="CASIMIR" \
      org.opencontainers.image.description="Architecture as code, validated like flight software" \
      org.opencontainers.image.source="https://github.com/casimirex/casimir" \
      org.opencontainers.image.licenses="Apache-2.0"

COPY --from=builder /build/target/release/casm /usr/local/bin/casm
COPY --from=builder /build/target/release/casm-lsp /usr/local/bin/casm-lsp

# Already non-root by way of the `:nonroot` tag; stated explicitly so a future base-image
# change cannot silently promote the container to root.
USER nonroot

WORKDIR /work
ENTRYPOINT ["/usr/local/bin/casm"]
CMD ["--help"]
