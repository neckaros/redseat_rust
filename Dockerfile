FROM rust:1.67 AS builder
WORKDIR /usr/src/redseat-rust
COPY . .
RUN cargo install --path .

FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y ffmpeg imagemagick && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/redseat-rust /usr/local/bin/redseat-rust
CMD ["redseat-rust"]