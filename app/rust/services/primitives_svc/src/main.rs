//! Primitives gRPC Service — единственный источник правды для генерации ключей и хешей.
//!
//! Port: 50052 (analytics_svc занимает 50051)
//!
//! Экспонирует:
//!   content_hash_v1  — BLAKE3 hex (primitives::hash)
//!   normalize_key    — context_key / concept_key нормализация
//!   stable_id        — join(parts, "|") → blake3_hex
//!   normalize_url    — URL нормализация (primitives::url_norm)

use anyhow::Result;
use infrastructure::adapters::tonic_adapter::{async_trait, Request, Response, Server, Status};
use tracing::info;

pub mod proto {
    infrastructure::adapters::tonic_adapter::include_proto!("alegria.primitives.v1");
}

use proto::{
    primitives_service_server::{PrimitivesService, PrimitivesServiceServer},
    HashRequest, HashResponse,
    NormalizeRequest, NormalizeResponse,
    StableIdRequest, StableIdResponse,
    UrlRequest, UrlResponse,
    PingRequest, PingResponse,
};

#[derive(Default)]
struct PrimitivesHandler;

#[async_trait]
impl PrimitivesService for PrimitivesHandler {
    async fn content_hash(
        &self,
        req: Request<HashRequest>,
    ) -> Result<Response<HashResponse>, Status> {
        let hex = primitives::hash::content_hash_v1(&req.into_inner().text);
        Ok(Response::new(HashResponse { hex }))
    }

    async fn normalize_key(
        &self,
        req: Request<NormalizeRequest>,
    ) -> Result<Response<NormalizeResponse>, Status> {
        match primitives::context_key::normalize_context_key(&req.into_inner().raw) {
            Ok(normalized) => Ok(Response::new(NormalizeResponse {
                normalized,
                ok: true,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(NormalizeResponse {
                normalized: String::new(),
                ok: false,
                error: e.to_string(),
            })),
        }
    }

    async fn stable_id(
        &self,
        req: Request<StableIdRequest>,
    ) -> Result<Response<StableIdResponse>, Status> {
        let parts: Vec<&str> = req.get_ref().parts.iter().map(String::as_str).collect();
        let id = primitives::stable_id::stable_rule_instance_id(&parts);
        Ok(Response::new(StableIdResponse { id }))
    }

    async fn normalize_url(
        &self,
        req: Request<UrlRequest>,
    ) -> Result<Response<UrlResponse>, Status> {
        let raw = req.into_inner().raw_url;
        if raw.is_empty() {
            return Ok(Response::new(UrlResponse {
                normalized: String::new(),
                ok: false,
                error: "empty url".into(),
            }));
        }
        let normalized = primitives::url_norm::url_norm(&raw);
        Ok(Response::new(UrlResponse {
            normalized,
            ok: true,
            error: String::new(),
        }))
    }

    async fn ping(
        &self,
        _req: Request<PingRequest>,
    ) -> Result<Response<PingResponse>, Status> {
        Ok(Response::new(PingResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().json().init();

    let port = std::env::var("PRIMITIVES_SVC_PORT").unwrap_or_else(|_| "50052".to_string());
    let addr = format!("0.0.0.0:{port}").parse()?;

    info!(addr = %addr, "primitives_svc starting");

    Server::builder()
        .add_service(PrimitivesServiceServer::new(PrimitivesHandler))
        .serve(addr)
        .await?;

    Ok(())
}
