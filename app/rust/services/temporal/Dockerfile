FROM rust:1.91-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    libprotobuf-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY app ./app

ENV SQLX_OFFLINE=true

RUN cargo build --release --locked \
    --manifest-path /app/app/rust/Cargo.toml \
    -p temporal_worker --bin temporal_worker

# The same Temporal worker owns native Cloudflare static release deployment.
# Node is required only for the pinned official cf CLI; no extra container.
FROM node:24-bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates python3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/app/rust/target/release/temporal_worker /usr/local/bin/temporal_worker
COPY infra/cloudflare/seo-static /app/infra/cloudflare/seo-static
COPY automation/deploy_seo_static_cloudflare.py \
     automation/check_public_static_output.py \
     automation/check_public_http_readback.py /app/automation/
RUN npm install --prefix /app/infra/cloudflare/seo-static --no-audit --no-fund

ENTRYPOINT ["/usr/local/bin/temporal_worker"]
