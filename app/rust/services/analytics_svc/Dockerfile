FROM rust:1.100.0-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY app ./app

ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=0
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256
ENV CARGO_PROFILE_RELEASE_STRIP=symbols

RUN cargo build --release --locked \
    --manifest-path /app/app/rust/Cargo.toml \
    -p analytics_svc

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY --from=builder /app/app/rust/target/release/analytics_svc /usr/local/bin/analytics_svc

ENV GRPC_PORT=50051
EXPOSE 50051

CMD ["analytics_svc"]
