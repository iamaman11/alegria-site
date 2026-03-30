FROM rust:1.88-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler pkg-config && rm -rf /var/lib/apt/lists/*

COPY app ./app

RUN cargo build --release \
    --manifest-path /app/app/rust/Cargo.toml \
    -p analytics_svc

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY --from=builder /app/app/rust/target/release/analytics_svc /usr/local/bin/analytics_svc

ENV GRPC_PORT=50051
EXPOSE 50051

CMD ["analytics_svc"]
