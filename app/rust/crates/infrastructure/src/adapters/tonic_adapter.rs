use anyhow::Result;
use std::time::Duration;
use tonic::transport::Channel;

pub use tonic::async_trait;
pub use tonic::include_proto;
pub use tonic::transport::Server;
pub use tonic::{Request, Response, Status};

pub mod proto {
    tonic::include_proto!("alegria.analytics.v1");
}
use proto::graph_analytics_service_client::GraphAnalyticsServiceClient;
pub use proto::{
    LinkPlanRequest, LinkPlanResult, PageRankRequest, PageRankResult, WccRequest, WccResult,
};

pub struct AnalyticsClient {
    inner: GraphAnalyticsServiceClient<Channel>,
}

impl AnalyticsClient {
    pub async fn connect(addr: &str) -> Result<Self> {
        let channel = Channel::from_shared(addr.to_string())?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(60))
            .connect()
            .await?;
        Ok(Self {
            inner: GraphAnalyticsServiceClient::new(channel),
        })
    }

    pub async fn run_wcc_optional(
        &mut self,
        graph_name: &str,
        min_cluster: i32,
    ) -> Option<WccResult> {
        let req = WccRequest {
            graph_name: graph_name.to_string(),
            min_cluster,
        };
        match self.inner.run_wcc(req).await {
            Ok(r) => Some(r.into_inner()),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Analytics service unavailable — skipping WCC clustering"
                );
                None
            }
        }
    }

    pub async fn build_link_plan(
        &mut self,
        context_key: &str,
        max_links: i32,
    ) -> Result<LinkPlanResult> {
        let req = LinkPlanRequest {
            context_key: context_key.to_string(),
            max_links,
        };
        Ok(self.inner.build_link_plan(req).await?.into_inner())
    }

    pub async fn ping(&mut self) -> bool {
        self.inner.ping(proto::PingRequest {}).await.is_ok()
    }
}
