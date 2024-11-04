FROM rust:1.82 AS builder
RUN apt-get update && apt-get install -y nasm && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/redseat-daemon
COPY . .
RUN cargo install --path .

FROM alpine:3.20.3
RUN apt-get update && apt-get install -y ffmpeg imagemagick && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/redseat-rust /usr/local/bin/redseat-rust
COPY --from=builder /usr/local/cargo/bin/redseat-daemon /usr/local/bin/redseat-daemon
RUN mkdir -p /config
run mkdir -p /libraries

VOLUME /config
VOLUME /libraries

EXPOSE 8080
CMD ["redseat-daemon", "--docker"]