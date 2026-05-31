use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::adapters::reqwest_adapter::new_default_client;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoyageInputType {
    Query,
    Document,
}

impl VoyageInputType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Document => "document",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoyageOutputDtype {
    Float,
    Int8,
    Uint8,
    Binary,
    Ubinary,
}

impl VoyageOutputDtype {
    fn as_str(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Int8 => "int8",
            Self::Uint8 => "uint8",
            Self::Binary => "binary",
            Self::Ubinary => "ubinary",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoyageEmbeddingOptions {
    pub input_type: Option<VoyageInputType>,
    pub output_dimension: Option<u32>,
    pub output_dtype: Option<VoyageOutputDtype>,
    pub truncation: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoyageRerankOptions {
    pub top_k: Option<usize>,
    pub truncation: Option<bool>,
    pub return_documents: bool,
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    truncation: Option<bool>,
}

#[derive(Serialize)]
struct VoyageContextualizedRequest<'a> {
    inputs: &'a [Vec<&'a str>],
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dimension: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dtype: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncation: Option<bool>,
}

#[derive(Serialize)]
struct VoyageRerankRequest<'a> {
    query: &'a str,
    documents: &'a [&'a str],
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<usize>,
    return_documents: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncation: Option<bool>,
}

#[derive(Deserialize)]
struct VoyageData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageData>,
}

#[derive(Deserialize)]
struct VoyageContextualizedResult {
    embeddings: Vec<Vec<f32>>,
    index: usize,
}

#[derive(Deserialize)]
struct VoyageContextualizedResponse {
    results: Vec<VoyageContextualizedResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoyageRerankEntry {
    pub index: usize,
    pub relevance_score: f32,
    #[serde(default)]
    pub document: Option<String>,
}

#[derive(Deserialize)]
struct VoyageRerankResponse {
    data: Vec<VoyageRerankEntry>,
}

pub struct VoyageClient {
    http: Client,
    api_key: String,
    model: String,
    rerank_model: String,
    base_url: String,
    limiter: Arc<Semaphore>,
}

impl VoyageClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let base_url = std::env::var("VOYAGE_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.voyageai.com/v1".to_string());
        Self {
            http: new_default_client(30).expect("reqwest client"),
            api_key: api_key.into(),
            model: model.into(),
            rerank_model: std::env::var("VOYAGE_RERANK_MODEL")
                .unwrap_or_else(|_| "rerank-2.5".to_string()),
            base_url,
            limiter: Arc::new(Semaphore::new(5)),
        }
    }

    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.embed_batch_with_settings(texts, &VoyageEmbeddingOptions::default())
            .await
    }

    pub async fn embed_batch_with_options(
        &self,
        texts: &[&str],
        input_type: Option<VoyageInputType>,
        output_dimension: Option<u32>,
        output_dtype: Option<VoyageOutputDtype>,
    ) -> Result<Vec<Vec<f32>>> {
        self.embed_batch_with_settings(
            texts,
            &VoyageEmbeddingOptions {
                input_type,
                output_dimension,
                output_dtype,
                truncation: None,
            },
        )
        .await
    }

    pub async fn embed_batch_with_settings(
        &self,
        texts: &[&str],
        options: &VoyageEmbeddingOptions,
    ) -> Result<Vec<Vec<f32>>> {
        if matches!(
            options.output_dtype,
            Some(VoyageOutputDtype::Int8)
                | Some(VoyageOutputDtype::Uint8)
                | Some(VoyageOutputDtype::Binary)
                | Some(VoyageOutputDtype::Ubinary)
        ) {
            anyhow::bail!(
                "non-float Voyage embeddings are not enabled in the primary quality path"
            );
        }
        let _permit = self.limiter.acquire().await?;
        let resp = self
            .http
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&VoyageRequest {
                input: texts,
                model: &self.model,
                input_type: options.input_type.map(VoyageInputType::as_str),
                output_dimension: options.output_dimension,
                output_dtype: options.output_dtype.map(VoyageOutputDtype::as_str),
                truncation: options.truncation,
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
        self.embed_all_with_settings(texts, &VoyageEmbeddingOptions::default())
            .await
    }

    pub async fn embed_all_with_options(
        &self,
        texts: &[String],
        input_type: Option<VoyageInputType>,
        output_dimension: Option<u32>,
        output_dtype: Option<VoyageOutputDtype>,
    ) -> Result<Vec<Vec<f32>>> {
        self.embed_all_with_settings(
            texts,
            &VoyageEmbeddingOptions {
                input_type,
                output_dimension,
                output_dtype,
                truncation: None,
            },
        )
        .await
    }

    pub async fn embed_all_with_settings(
        &self,
        texts: &[String],
        options: &VoyageEmbeddingOptions,
    ) -> Result<Vec<Vec<f32>>> {
        let mut result = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(128) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let mut batch = self.embed_batch_with_settings(&refs, options).await?;
            result.append(&mut batch);
        }
        Ok(result)
    }

    pub async fn contextualized_embed(
        &self,
        inputs: &[Vec<&str>],
        options: &VoyageEmbeddingOptions,
        model: Option<&str>,
    ) -> Result<Vec<Vec<Vec<f32>>>> {
        if matches!(
            options.output_dtype,
            Some(VoyageOutputDtype::Int8)
                | Some(VoyageOutputDtype::Uint8)
                | Some(VoyageOutputDtype::Binary)
                | Some(VoyageOutputDtype::Ubinary)
        ) {
            anyhow::bail!(
                "non-float contextualized Voyage embeddings are not enabled in the primary quality path"
            );
        }
        let _permit = self.limiter.acquire().await?;
        let resp = self
            .http
            .post(format!("{}/contextualizedembeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&VoyageContextualizedRequest {
                inputs,
                model: model.unwrap_or(&self.model),
                input_type: options.input_type.map(VoyageInputType::as_str),
                output_dimension: options.output_dimension,
                output_dtype: options.output_dtype.map(VoyageOutputDtype::as_str),
                truncation: options.truncation,
            })
            .send()
            .await?
            .error_for_status()
            .context("Voyage contextualized embeddings HTTP error")?
            .json::<VoyageContextualizedResponse>()
            .await
            .context("Voyage contextualized embeddings parse error")?;

        let mut results = vec![Vec::new(); inputs.len()];
        for entry in resp.results {
            if entry.index < results.len() {
                results[entry.index] = entry.embeddings;
            }
        }
        Ok(results)
    }

    pub async fn rerank(
        &self,
        query: &str,
        documents: &[&str],
        options: &VoyageRerankOptions,
        model: Option<&str>,
    ) -> Result<Vec<VoyageRerankEntry>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let _permit = self.limiter.acquire().await?;
        let resp = self
            .http
            .post(format!("{}/rerank", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&VoyageRerankRequest {
                query,
                documents,
                model: model.unwrap_or(&self.rerank_model),
                top_k: options.top_k,
                return_documents: options.return_documents,
                truncation: options.truncation,
            })
            .send()
            .await?
            .error_for_status()
            .context("Voyage rerank HTTP error")?
            .json::<VoyageRerankResponse>()
            .await
            .context("Voyage rerank parse error")?;
        Ok(resp.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_type_and_dtype_serialize_lowercase() {
        let payload = serde_json::to_value(VoyageEmbeddingOptions {
            input_type: Some(VoyageInputType::Document),
            output_dimension: Some(1024),
            output_dtype: Some(VoyageOutputDtype::Float),
            truncation: Some(false),
        })
        .unwrap();
        assert_eq!(payload["input_type"], "document");
        assert_eq!(payload["output_dtype"], "float");
        assert_eq!(payload["truncation"], false);
    }
}
