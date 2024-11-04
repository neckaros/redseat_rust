FROM rust:alpine AS builder
WORKDIR /app/src
RUN USER=root
RUN apk add pkgconfig openssl-dev libc-dev nasm
COPY . .
RUN cargo build --release

FROM alpine:3.20.3
WORKDIR /app
RUN apk add --no-cache openssl ca-certificates ffmpeg imagemagick
COPY --from=builder /app/src/target/release/redseat-rust /app/redseat-rust
COPY --from=builder /app/src/target/release/redseat-daemon /app/redseat-daemon
RUN mkdir -p /config
RUN mkdir -p /libraries

VOLUME /config
VOLUME /libraries

EXPOSE 8080
CMD ["redseat-daemon", "--docker"]