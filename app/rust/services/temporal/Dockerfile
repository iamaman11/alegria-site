FROM rust:1.100.0-slim-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    libprotobuf-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

ENV SQLX_OFFLINE=true
# Experimental deployment: ship native binaries with a fast Rust codegen
# profile. Optimized release codegen was the cold Docker CI bottleneck.
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=0
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256
ENV CARGO_PROFILE_RELEASE_STRIP=symbols

# CI generates a stable, source-free Cargo skeleton from the *actual* workspace
# metadata. This native Cargo step caches third-party .rlib dependencies in an
# ordinary Docker layer, so `cache-to: type=gha,mode=max` exports the objects
# to future GitHub runners. BuildKit cache mounts alone would NOT survive.
COPY dependency-skeleton/app ./app
RUN cargo build --release --locked \
    --manifest-path /app/app/rust/Cargo.toml \
    -p temporal_worker -p cli_tools \
    --bin temporal_worker --bin temporal_starter --bin cli_tools

# A production Rust edit invalidates only this second COPY and actual codegen,
# not the preceding locked dependency compilation layer. Real build.rs files,
# generated-protobuf inputs and .sqlx queries enter only in this second phase.
COPY app ./app

# Build the real Worker, Starter and native CMS CLI in the SAME Cargo graph.
# Touch real Rust sources/build scripts to make Cargo's timestamp-sensitive freshness
# checks independent of Docker COPY preserving older checkout mtimes.
RUN find /app/app/rust -type f -name '*.rs' \
    \( -path '*/src/*' -o -name 'build.rs' \) -exec touch {} + \
    && cargo build --release --locked \
    --manifest-path /app/app/rust/Cargo.toml \
    -p temporal_worker -p cli_tools \
    --bin temporal_worker --bin temporal_starter --bin cli_tools

# The same Temporal worker owns native Cloudflare static release deployment.
# Node is required only for the pinned official cf CLI; no extra container.
FROM node:26.13.1-bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates python3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Native Cloudflare tooling changes only with its package manifest. Keep npm
# ahead of the Rust binaries, website assets and source SHA: neither a Worker
# rebuild nor an assets-only release should reinstall identical dependencies.
COPY infra/cloudflare/seo-static/package.json /app/infra/cloudflare/seo-static/package.json
RUN npm install --prefix /app/infra/cloudflare/seo-static --no-audit --no-fund

# Website config/assets and the exact-source release envelope are independent
# of the compiler and Node dependency layers. BuildKit COPY --link keeps their
# layers independent of earlier changes, and can rebase the unchanged native
# binaries instead of exporting them again for assets/provenance-only releases.
# These destinations are ordinary directories/files, never symlink targets.
COPY --link infra/cloudflare/seo-static /app/infra/cloudflare/seo-static
COPY --link automation/deploy_seo_static_cloudflare.py \
     automation/check_public_static_output.py \
     automation/check_public_http_readback.py \
     automation/check_first_site_release.py /app/automation/
COPY --link worker-context.json /app/worker-context.json
COPY --link --from=builder /app/app/rust/target/release/temporal_worker /usr/local/bin/temporal_worker
COPY --link --from=builder /app/app/rust/target/release/temporal_starter /usr/local/bin/temporal_starter
COPY --link --from=builder /app/app/rust/target/release/cli_tools /usr/local/bin/cli_tools

ENTRYPOINT ["/usr/local/bin/temporal_worker"]
