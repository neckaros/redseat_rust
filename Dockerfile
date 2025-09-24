# Build stage
FROM ubuntu:24.04 AS builderimage

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y software-properties-common

RUN add-apt-repository ppa:ubuntuhandbook1/libheif

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    git \
    pkg-config \
    libde265-dev \
    libheif-dev \
    libwebp-dev \
    libjpeg-dev \
    libpng-dev \
    libtiff-dev \
    libzip-dev \
    libltdl-dev \
    libraw-dev \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Server build stage
FROM rust:1.90 AS builder
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
FROM debian:bookworm-slim

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
    && rm -rf /var/lib/apt/lists/*

# Copy ImageMagick files from builder
COPY --from=builderimage /usr/local/lib /usr/local/lib
COPY --from=builderimage /usr/local/bin /usr/local/bin
COPY --from=builderimage /usr/local/etc /usr/local/etc
COPY --from=builderimage /usr/local/include /usr/local/include
COPY --from=builderimage /usr/local/share /usr/local/share

# Update library cache
RUN ldconfig

RUN apt-get update && apt-get install -y ffmpeg && rm -rf /var/lib/apt/lists/*

RUN apt-get -y purge software-properties-common

WORKDIR /app
COPY --from=builder /usr/src/redseat-daemon/target/release/redseat-rust /app/redseat-rust
COPY --from=builder /usr/src/redseat-daemon/target/release/redseat-daemon /app/redseat-daemon
EXPOSE 8080
CMD ["./redseat-daemon", "--docker"]
