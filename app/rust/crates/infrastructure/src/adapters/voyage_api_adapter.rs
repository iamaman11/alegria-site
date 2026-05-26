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
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dimension: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dtype: Option<&'a str>,
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
        self.embed_batch_with_options(texts, None, None, None).await
    }

    pub async fn embed_batch_with_options(
        &self,
        texts: &[&str],
        input_type: Option<&str>,
        output_dimension: Option<u32>,
        output_dtype: Option<&str>,
    ) -> Result<Vec<Vec<f32>>> {
        let _permit = self.limiter.acquire().await?;
        let resp = self
            .http
            .post("https://api.voyageai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&VoyageRequest {
                input: texts,
                model: &self.model,
                input_type,
                output_dimension,
                output_dtype,
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
        self.embed_all_with_options(texts, None, None, None).await
    }

    pub async fn embed_all_with_options(
        &self,
        texts: &[String],
        input_type: Option<&str>,
        output_dimension: Option<u32>,
        output_dtype: Option<&str>,
    ) -> Result<Vec<Vec<f32>>> {
        let mut result = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(128) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let mut batch = self
                .embed_batch_with_options(&refs, input_type, output_dimension, output_dtype)
                .await?;
            result.append(&mut batch);
        }
        Ok(result)
    }
}
