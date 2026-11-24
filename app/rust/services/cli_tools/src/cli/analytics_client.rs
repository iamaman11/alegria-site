//! Client used only by the operator's analytics health probe.

use anyhow::Result;
use std::time::Duration;
use tonic::transport::Channel;

mod proto {
    tonic::include_proto!("alegria.analytics.v1");
}

use proto::graph_analytics_service_client::GraphAnalyticsServiceClient;

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

    pub async fn ping(&mut self) -> bool {
        self.inner.ping(proto::PingRequest {}).await.is_ok()
    }
}
