FROM debian:bookworm-slim AS builderimage

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
    libjpeg62-turbo-dev \
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
FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive

# Install only required runtime libraries
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        gcc-12-base \
        libc6 \
        libfreetype6 \
        libfontconfig1 \
        libgomp1 \
        libheif1 \
        libjpeg62-turbo \
        liblcms2-2 \
        libltdl7 \
        libpng16-16 \
        libssl3 \
        libtiff5 \
        libwebp7 \
        libxml2 \
        zlib1g \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy ImageMagick build from builder
COPY --from=builderimage /install/usr/local /usr/local

# Update library cache
RUN ldconfig

RUN apt-get update && apt-get install -y ffmpeg && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /usr/local/cargo/bin/redseat-rust /app/redseat-rust
COPY --from=builder /usr/local/cargo/bin/redseat-daemon /app/redseat-daemon
EXPOSE 8080
CMD ["./redseat-daemon", "--docker"]