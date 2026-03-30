use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::adapters::reqwest_adapter::new_default_client;

#[derive(Serialize)]
struct VoyageRequest<'a> {
    input: &'a [&'a str],
    model: &'a str,
}

#[derive(Deserialize)]
struct VoyageData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageData>,
}

pub struct VoyageClient {
    http: Client,
    api_key: String,
    model: String,
    limiter: Arc<Semaphore>,
}

impl VoyageClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: new_default_client(30).expect("reqwest client"),
            api_key: api_key.into(),
            model: model.into(),
            limiter: Arc::new(Semaphore::new(5)),
        }
    }

    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let _permit = self.limiter.acquire().await?;
        let resp = self
            .http
            .post("https://api.voyageai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&VoyageRequest {
                input: texts,
                model: &self.model,
            })
            .send()
            .await?
            .error_for_status()
            .context("Voyage API HTTP error")?
            .json::<VoyageResponse>()
            .await
            .context("Voyage API response parse error")?;
        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    pub async fn embed_all(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut result = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(128) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let mut batch = self.embed_batch(&refs).await?;
            result.append(&mut batch);
        }
        Ok(result)
    }
}
