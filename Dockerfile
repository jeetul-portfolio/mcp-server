# syntax=docker/dockerfile:1.7
# Cross-compiles a static musl binary on the build host (no QEMU for the Rust build),
# then ships it in a distroless, non-root image.

FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.6.1 AS xx

FROM --platform=$BUILDPLATFORM rust:1.97-alpine AS builder
COPY --from=xx / /
RUN apk add --no-cache clang lld
ARG TARGETPLATFORM
RUN xx-apk add --no-cache musl-dev gcc

WORKDIR /src

# Cache dependencies separately from sources.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && touch src/lib.rs \
    && xx-cargo build --release --locked \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs src/lib.rs \
    && xx-cargo build --release --locked \
    && BIN="target/$(xx-cargo --print-target-triple)/release/portfolio-mcp" \
    && xx-verify --static "$BIN" \
    && cp "$BIN" /portfolio-mcp

FROM gcr.io/distroless/static-debian12:nonroot

COPY --from=builder /portfolio-mcp /usr/local/bin/portfolio-mcp

# The deployed configuration ships inside the image. Binding 0.0.0.0 is refused unless that
# file sets allow_remote and behind_tls_proxy.
COPY --chown=65532:65532 config.toml /app/config.toml

EXPOSE 8080
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/portfolio-mcp", "--config", "/app/config.toml"]
