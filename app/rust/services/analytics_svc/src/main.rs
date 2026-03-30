use anyhow::Result;
use infrastructure::adapters::neo4rs_adapter::{connect_neo4j, query, Graph};
use infrastructure::adapters::tonic_adapter::{async_trait, Request, Response, Server, Status};
use std::collections::{HashMap, HashSet};
use tracing::info;

pub mod proto {
    infrastructure::adapters::tonic_adapter::include_proto!("alegria.analytics.v1");
}

use proto::graph_analytics_service_server::{GraphAnalyticsService, GraphAnalyticsServiceServer};
use proto::{
    LinkPlanRequest, LinkEntry, LinkPlanResult, PageRankEntry, PageRankRequest, PageRankResult, PingRequest,
    PingResponse, WccRequest, WccResult,
};

#[derive(Clone)]
struct AnalyticsSvc {
    graph: Graph,
}

impl AnalyticsSvc {
    async fn ensure_gds_available(&self) -> Result<String> {
        let mut rows = self
            .graph
            .execute(query("RETURN gds.version() AS version"))
            .await?;

        while let Ok(Some(row)) = rows.next().await {
            if let Ok(version) = row.get::<String>("version") {
                return Ok(version);
            }
        }

        anyhow::bail!("GDS procedure available but version row is empty");
    }

    async fn ensure_graph_projection(&self, graph_name: &str) -> Result<()> {
        let mut result = self
            .graph
            .execute(query("CALL gds.graph.exists($name) YIELD exists RETURN exists").param("name", graph_name))
            .await?;

        let mut exists = false;
        while let Ok(Some(row)) = result.next().await {
            if let Ok(v) = row.get::<bool>("exists") {
                exists = v;
            }
        }

        if !exists {
            self.graph
                .run(
                    query(
                        "CALL gds.graph.project(
                           $name,
                           'Context',
                           { RELATES_TO: { orientation: 'UNDIRECTED' } }
                         )",
                    )
                    .param("name", graph_name),
                )
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl GraphAnalyticsService for AnalyticsSvc {
    async fn ping(&self, _req: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        Ok(Response::new(PingResponse {
            version: "1.0.0-rust".to_string(),
        }))
    }

    async fn run_wcc(&self, req: Request<WccRequest>) -> Result<Response<WccResult>, Status> {
        let req = req.into_inner();
        let graph_name = if req.graph_name.is_empty() {
            "alegria_graph".to_string()
        } else {
            req.graph_name
        };

        self.ensure_graph_projection(&graph_name)
            .await
            .map_err(|e| Status::internal(format!("ensure projection failed: {e}")))?;

        let q = query(
            "CALL gds.wcc.stream($graph_name)
             YIELD nodeId, componentId
             RETURN gds.util.asNode(nodeId).key AS context_key, componentId",
        )
        .param("graph_name", graph_name.clone());

        let mut rows = self
            .graph
            .execute(q)
            .await
            .map_err(|e| Status::internal(format!("wcc failed: {e}")))?;

        let mut assignments: HashMap<String, i64> = HashMap::new();
        let mut unique_clusters: HashSet<i64> = HashSet::new();

        while let Ok(Some(row)) = rows.next().await {
            let context_key = row
                .get::<String>("context_key")
                .map_err(|e| Status::internal(format!("invalid context_key row: {e}")))?;
            let component_id = row
                .get::<i64>("componentId")
                .map_err(|e| Status::internal(format!("invalid componentId row: {e}")))?;
            assignments.insert(context_key, component_id);
            unique_clusters.insert(component_id);
        }

        let status = if assignments.is_empty() { "graph_empty" } else { "ok" }.to_string();
        let cluster_count = unique_clusters.len() as i32;

        Ok(Response::new(WccResult {
            assignments,
            cluster_count,
            status,
        }))
    }

    async fn compute_page_rank(
        &self,
        req: Request<PageRankRequest>,
    ) -> Result<Response<PageRankResult>, Status> {
        let req = req.into_inner();
        let graph_name = if req.graph_name.is_empty() {
            "alegria_graph".to_string()
        } else {
            req.graph_name
        };
        let top_n = if req.top_n <= 0 { 100 } else { req.top_n };
        let damping = if req.damping <= 0.0 { 0.85 } else { req.damping };

        self.ensure_graph_projection(&graph_name)
            .await
            .map_err(|e| Status::internal(format!("ensure projection failed: {e}")))?;

        let q = query(
            "CALL gds.pageRank.stream($graph_name, {maxIterations: 20, dampingFactor: $damping})
             YIELD nodeId, score
             RETURN gds.util.asNode(nodeId).key AS context_key, score
             ORDER BY score DESC LIMIT $limit",
        )
        .param("graph_name", graph_name)
        .param("damping", damping as f64)
        .param("limit", top_n as i64);

        let mut rows = self
            .graph
            .execute(q)
            .await
            .map_err(|e| Status::internal(format!("pagerank failed: {e}")))?;

        let mut entries = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let context_key = row
                .get::<String>("context_key")
                .map_err(|e| Status::internal(format!("invalid context_key row: {e}")))?;
            let score_f64 = row
                .get::<f64>("score")
                .map_err(|e| Status::internal(format!("invalid score row: {e}")))?;
            entries.push(PageRankEntry {
                context_key,
                score: score_f64 as f32,
            });
        }

        Ok(Response::new(PageRankResult { entries }))
    }

    async fn build_link_plan(
        &self,
        req: Request<LinkPlanRequest>,
    ) -> Result<Response<LinkPlanResult>, Status> {
        let req = req.into_inner();
        if req.context_key.is_empty() {
            return Err(Status::invalid_argument("context_key is required"));
        }
        let max_links = if req.max_links <= 0 { 5 } else { req.max_links } as i64;

        let q = query(
            "MATCH (c:Context {key: $key})-[:BELONGS_TO]->(concept:Concept)
             MATCH (other:Context)-[:BELONGS_TO]->(concept)
             WHERE c <> other
             RETURN other.key as to_key, count(*) as weight
             ORDER BY weight DESC LIMIT $limit",
        )
        .param("key", req.context_key.clone())
        .param("limit", max_links);

        let mut rows = self
            .graph
            .execute(q)
            .await
            .map_err(|e| Status::internal(format!("build_link_plan failed: {e}")))?;

        let mut links = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            let to_key = row
                .get::<String>("to_key")
                .map_err(|e| Status::internal(format!("invalid to_key row: {e}")))?;
            let weight = row
                .get::<i64>("weight")
                .map_err(|e| Status::internal(format!("invalid weight row: {e}")))?;
            links.push(LinkEntry {
                from_key: req.context_key.clone(),
                to_key,
                relevance_score: weight as f32,
            });
        }

        Ok(Response::new(LinkPlanResult { links }))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().json().init();

    let neo4j_uri = std::env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
    let neo4j_user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let neo4j_password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());
    let port = std::env::var("GRPC_PORT").unwrap_or_else(|_| "50051".to_string());
    let addr = format!("0.0.0.0:{port}").parse()?;

    let graph = connect_neo4j(&neo4j_uri, &neo4j_user, &neo4j_password).await?;
    let svc = AnalyticsSvc { graph };

    let gds_version = svc.ensure_gds_available().await?;
    info!(%gds_version, "neo4j GDS check passed");
    info!(%addr, "analytics_svc (Rust) starting");

    Server::builder()
        .add_service(GraphAnalyticsServiceServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
