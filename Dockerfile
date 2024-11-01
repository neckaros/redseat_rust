FROM rust:1.82 AS builder
RUN apt-get update && apt-get install -y nasm && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/redseat-daemon
COPY . .
RUN cargo install --path .

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ffmpeg imagemagick && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/redseat-daemon /usr/local/bin/redseat-daemon
EXPOSE 8080
CMD ["redseat-daemon", "--docker"]