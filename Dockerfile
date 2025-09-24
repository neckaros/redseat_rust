# Server build stage
FROM rust:1.90-trixie AS builder
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    build-essential \
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
    libjpeg62-turbo \
    libde265-0 \
    libheif1 \
    libaom3 \
    libdav1d6 \
    libx265-199 \
    libwebp7 \
    libpng16-16 \
    libtiff6 \
    libzip4 \
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
