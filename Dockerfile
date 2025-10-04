# Server build stage
FROM rust:1.90-trixie AS builder
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    build-essential \
    libheif-dev \
    nasm \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/redseat-daemon
COPY . .
RUN cargo build --release


# Run stage
FROM debian:trixie-slim

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    # Core libheif runtime from trixie
    libheif1 \
    # Decoding plugin(s) for HEIC/HEVC
    libheif-plugin-libde265 \
    libde265-0 \
    libjpeg62-turbo \
    libaom3 \
    libdav1d7 \
    libx265-215 \
    libwebp7 \
    libpng16-16t64 \
    libtiff6 \
    libzip5 \
    libltdl7 \
    libgomp1 \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

# Update library cache
RUN ldconfig


WORKDIR /app
COPY --from=builder /usr/src/redseat-daemon/target/release/redseat-rust /app/redseat-rust
COPY --from=builder /usr/src/redseat-daemon/target/release/redseat-daemon /app/redseat-daemon
EXPOSE 8080
CMD ["./redseat-daemon", "--docker"]
