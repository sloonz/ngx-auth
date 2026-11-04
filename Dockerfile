FROM rust:1.92-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/* && mkdir /data
WORKDIR /opt/ngx-auth
COPY --from=builder /build/target/release/ngx-auth /usr/local/bin/
EXPOSE 3000
ENV NODE_ENV=production LISTEN=3000 DB_TYPE=sqlite3 DB_FILENAME=/data/db.sqlite
ENTRYPOINT ["ngx-auth"]
