FROM rust:1.82 AS builder
RUN apt-get update && apt-get install -y nasm && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/redseat-rust
COPY . .
RUN cargo install --path .

FROM debian:bookworm-slim
RUN add-apt-repository ppa:tomtomtom/yt-dlp
RUN apt-get update && apt-get install -y ffmpeg imagemagick && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/redseat-rust /usr/local/bin/redseat-rust
EXPOSE 8080
CMD ["redseat-rust"]