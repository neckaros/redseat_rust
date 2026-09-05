# Server build stage
FROM rust:1.90-trixie AS builder
RUN apt-get update && apt-get install -y \
    ca-certificates \
    cmake \
    curl \
    pkg-config \
    build-essential \
  && rm -rf /var/lib/apt/lists/*
COPY scripts/install-libheif.sh /tmp/install-libheif.sh
RUN bash /tmp/install-libheif.sh
WORKDIR /usr/src/redseat-daemon

# Cache dependencies — only invalidated when Cargo.toml/lock/build.rs change
COPY Cargo.toml Cargo.lock build.rs ./
RUN mkdir -p src/daemon \
    && echo 'fn main() {}' > src/main.rs \
    && echo 'fn main() {}' > src/daemon/main.rs \
    && cargo build --release \
    && rm -rf target/release/deps/redseat* target/release/redseat*

# Copy real source and rebuild only your code
COPY src/ src/
RUN cargo build --release


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
