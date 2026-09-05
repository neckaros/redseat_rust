# syntax=docker/dockerfile:1

# Rust toolchain shared by the application builder.
FROM rust:1.90-trixie AS rust-toolchain
ARG SCCACHE_VERSION=0.17.0
ARG SCCACHE_SHA256=67c4a96dd237c1f518f6b36083f270f9976d516f1e57fce891755ea782e50006
RUN apt-get update && apt-get install -y \
    ca-certificates \
    cmake \
    curl \
    pkg-config \
    build-essential \
    nasm \
  && rm -rf /var/lib/apt/lists/*
RUN curl --fail --location \
      --output /tmp/sccache.tar.gz \
      "https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    && printf '%s  %s\n' "${SCCACHE_SHA256}" /tmp/sccache.tar.gz | sha256sum --check \
    && tar --extract \
      --gzip \
      --file /tmp/sccache.tar.gz \
      --strip-components 1 \
      --directory /usr/local/bin \
      "sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache" \
    && rm /tmp/sccache.tar.gz

# Server build stage
FROM rust-toolchain AS builder
ENV SYSTEM_DEPS_LIBHEIF_LINK=static
COPY scripts/install-libheif.sh /tmp/install-libheif.sh
RUN bash /tmp/install-libheif.sh
WORKDIR /usr/src/redseat-daemon

# Cache this whole layer for unchanged manifests; sccache reuses unaffected crates
# when a manifest change requires Cargo to rebuild the layer.
COPY Cargo.toml Cargo.lock build.rs ./
RUN --mount=type=secret,id=sccache_gha_token \
    --mount=type=secret,id=sccache_gha_url \
    set -eu; \
    if [ -s /run/secrets/sccache_gha_token ] && [ -s /run/secrets/sccache_gha_url ]; then \
      export ACTIONS_RUNTIME_TOKEN="$(cat /run/secrets/sccache_gha_token)"; \
      export ACTIONS_RESULTS_URL="$(cat /run/secrets/sccache_gha_url)"; \
      export SCCACHE_GHA_ENABLED=on; \
      export SCCACHE_GHA_VERSION=redseat-rust-docker-v1; \
      export RUSTC_WRAPPER=sccache; \
    fi; \
    mkdir -p src/daemon; \
    echo 'fn main() {}' > src/main.rs; \
    echo 'fn main() {}' > src/daemon/main.rs; \
    cargo build --release; \
    if [ "${SCCACHE_GHA_ENABLED:-}" = on ]; then \
      sccache --stop-server; \
    fi; \
    rm -rf target/release/deps/redseat* target/release/redseat*

# Copy real source and rebuild only your code
COPY src/ src/
RUN --mount=type=secret,id=sccache_gha_token \
    --mount=type=secret,id=sccache_gha_url \
    set -eu; \
    if [ -s /run/secrets/sccache_gha_token ] && [ -s /run/secrets/sccache_gha_url ]; then \
      export ACTIONS_RUNTIME_TOKEN="$(cat /run/secrets/sccache_gha_token)"; \
      export ACTIONS_RESULTS_URL="$(cat /run/secrets/sccache_gha_url)"; \
      export SCCACHE_GHA_ENABLED=on; \
      export SCCACHE_GHA_VERSION=redseat-rust-docker-v1; \
      export RUSTC_WRAPPER=sccache; \
    fi; \
    cargo build --release; \
    if [ "${SCCACHE_GHA_ENABLED:-}" = on ]; then \
      sccache --stop-server; \
    fi


# Run stage
FROM debian:trixie-slim

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libdav1d7 \
    libjpeg62-turbo \
    libwebp7 \
    libpng16-16t64 \
    libtiff6 \
    libzip5 \
    libltdl7 \
    libgomp1 \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*


WORKDIR /app
COPY --from=builder /usr/src/redseat-daemon/target/release/redseat-rust /app/redseat-rust
COPY --from=builder /usr/src/redseat-daemon/target/release/redseat-daemon /app/redseat-daemon
EXPOSE 8080
CMD ["./redseat-daemon", "--docker"]
