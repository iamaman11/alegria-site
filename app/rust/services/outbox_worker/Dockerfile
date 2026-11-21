FROM rust:1.100.0-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    libprotobuf-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY app ./app

ENV SQLX_OFFLINE=true
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=0
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256
ENV CARGO_PROFILE_RELEASE_STRIP=symbols

RUN cargo build --release --locked \
    --manifest-path /app/app/rust/Cargo.toml \
    -p outbox_worker

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/app/rust/target/release/outbox_worker /usr/local/bin/outbox_worker

ENTRYPOINT ["/usr/local/bin/outbox_worker"]
