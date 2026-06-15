use std::time::Duration;

use primitives::errors::DomainError;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

const DEFAULT_VERTEX_PROJECT: &str = "alegria-site-prod";
const DEFAULT_VERTEX_LOCATION: &str = "global";
const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

pub(crate) fn vertex_project() -> String {
    std::env::var("VERTEX_GEMINI_PROJECT")
        .or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT"))
        .or_else(|_| std::env::var("GCLOUD_PROJECT"))
        .unwrap_or_else(|_| DEFAULT_VERTEX_PROJECT.to_string())
}

pub(crate) fn vertex_location() -> String {
    std::env::var("VERTEX_GEMINI_LOCATION")
        .or_else(|_| std::env::var("GOOGLE_CLOUD_LOCATION"))
        .unwrap_or_else(|_| DEFAULT_VERTEX_LOCATION.to_string())
}

pub(crate) fn vertex_model(var_name: &str, fallback: &str) -> String {
    std::env::var(var_name)
        .or_else(|_| std::env::var("VERTEX_GEMINI_MODEL"))
        .unwrap_or_else(|_| fallback.to_string())
}

pub(crate) fn vertex_provider_enabled() -> bool {
    std::env::var("VERTEX_GEMINI_ENABLED")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

pub(crate) fn vertex_provider_configured() -> bool {
    if !vertex_provider_enabled() {
        return false;
    }
    !vertex_project().trim().is_empty()
}

pub(crate) fn vertex_generate_url(model: &str) -> String {
    format!(
        "https://aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
        vertex_project(),
        vertex_location(),
        model
    )
}

pub(crate) async fn vertex_bearer_headers(
    timeout: Duration,
) -> Result<reqwest::header::HeaderMap, DomainError> {
    let token = vertex_access_token(timeout).await?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        "application/json"
            .parse()
            .map_err(|e| DomainError::InfraUnavailable {
                message: format!("invalid content-type header: {e}"),
            })?,
    );
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {token}")
            .parse()
            .map_err(|e| DomainError::InfraUnavailable {
                message: format!("invalid Vertex auth header: {e}"),
            })?,
    );
    Ok(headers)
}

async fn vertex_access_token(timeout: Duration) -> Result<String, DomainError> {
    if let Ok(token) = std::env::var("VERTEX_GEMINI_ACCESS_TOKEN") {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Ok(token) = metadata_server_access_token(timeout).await {
        return Ok(token);
    }

    gcloud_adc_access_token().await
}

async fn metadata_server_access_token(timeout: Duration) -> Result<String, DomainError> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| DomainError::InfraUnavailable {
            message: format!("vertex metadata http client build failed: {e}"),
        })?;
    let response = client
        .get(METADATA_TOKEN_URL)
        .header("Metadata-Flavor", "Google")
        .send()
        .await
        .map_err(|e| DomainError::InfraUnavailable {
            message: format!("vertex metadata token request failed: {e}"),
        })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| DomainError::InfraUnavailable {
            message: format!("vertex metadata token read failed: {e}"),
        })?;
    if !status.is_success() {
        return Err(DomainError::InfraUnavailable {
            message: format!("vertex metadata token status={} body={}", status, text),
        });
    }
    let value =
        serde_json::from_str::<Value>(&text).map_err(|e| DomainError::InfraUnavailable {
            message: format!("vertex metadata token JSON decode failed: {e}; body={text}"),
        })?;
    value
        .get("access_token")
        .and_then(Value::as_str)
        .map(|token| token.to_string())
        .ok_or_else(|| DomainError::InfraUnavailable {
            message: "vertex metadata token response missing access_token".to_string(),
        })
}

async fn gcloud_adc_access_token() -> Result<String, DomainError> {
    let output = tokio::process::Command::new("gcloud")
        .args(["auth", "application-default", "print-access-token"])
        .output()
        .await
        .map_err(|e| DomainError::InfraUnavailable {
            message: format!("gcloud ADC token command failed: {e}"),
        })?;
    if !output.status.success() {
        return Err(DomainError::InfraUnavailable {
            message: format!(
                "gcloud ADC token command status={} stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    let token = String::from_utf8(output.stdout).map_err(|e| DomainError::InfraUnavailable {
        message: format!("gcloud ADC token stdout decode failed: {e}"),
    })?;
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(DomainError::InfraUnavailable {
            message: "gcloud ADC token command returned empty access token".to_string(),
        });
    }
    Ok(trimmed.to_string())
}
