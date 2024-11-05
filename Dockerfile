# IamgeMagick Build stage
FROM ubuntu:latest AS builderimagemagick

ENV DEBIAN_FRONTEND=noninteractive
ENV IMAGEMAGICK_VERSION=7.1.1-29

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    git \
    pkg-config \
    libde265-dev \
    libheif-dev \
    libwebp-dev \
    libpng-dev \
    libjpeg-dev \
    libtiff-dev \
    libxml2-dev \
    libssl-dev \
    libfreetype6-dev \
    libfontconfig1-dev \
    libltdl7-dev \
    liblcms2-dev \
    libgomp1 \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Download and compile ImageMagick
WORKDIR /tmp
RUN wget https://github.com/ImageMagick/ImageMagick/archive/${IMAGEMAGICK_VERSION}.tar.gz && \
    tar xvzf ${IMAGEMAGICK_VERSION}.tar.gz && \
    cd ImageMagick-${IMAGEMAGICK_VERSION} && \
    ./configure \
        --with-heic=yes \
        --with-webp=yes \
        --with-jpeg=yes \
        --with-png=yes \
        --with-tiff=yes \
        --enable-shared \
        --disable-static \
        --with-modules \
        --enable-openmp \
        --prefix=/usr/local \
        --disable-docs \
        --disable-deprecated \
        --disable-hdri \
        --without-perl \
        --without-magick-plus-plus \
        --without-x && \
    make -j$(nproc) && \
    make install DESTDIR=/install && \
    cd .. && \
    rm -rf ImageMagick-${IMAGEMAGICK_VERSION} ${IMAGEMAGICK_VERSION}.tar.gz

# Server build stage
FROM rust:1.82 AS builder
RUN apt-get update && apt-get install -y nasm && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/redseat-daemon
COPY . .
RUN cargo install --path .



# Run stage
FROM ubuntu:latest

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install only required runtime libraries
RUN apt-get update && apt-get install -y --no-install-recommends \
    libde265-0 \
    libheif1 \
    libwebp7 \
    libpng16-16 \
    libjpeg8 \
    libtiff5 \
    libxml2 \
    libssl3 \
    libfreetype6 \
    libfontconfig1 \
    libltdl7 \
    liblcms2-2 \
    libgomp1 \
    && rm -rf /var/lib/apt/lists/*

# Copy ImageMagick build from builder
COPY --from=builderimagemagick /install/usr/local /usr/local

# Update library cache
RUN ldconfig


RUN apt-get update && apt-get install -y ffmpeg && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /usr/local/cargo/bin/redseat-rust /app/redseat-rust
COPY --from=builder /usr/local/cargo/bin/redseat-daemon /app/redseat-daemon
EXPOSE 8080
CMD ["./redseat-daemon", "--docker"]