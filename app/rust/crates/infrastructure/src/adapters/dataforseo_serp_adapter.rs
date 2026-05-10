use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

use primitives::url_norm::{domain_norm, url_norm};

use super::reqwest_adapter;

const DEFAULT_ENDPOINT: &str = "https://api.dataforseo.com/v3/serp/google/organic/live/advanced";

#[derive(Debug, Clone)]
pub struct DataForSeoConfig {
    pub endpoint: String,
    pub login: String,
    pub password: String,
    pub language_code: String,
    pub location_code: i64,
    pub depth: u32,
}

#[derive(Debug, Clone)]
pub struct DataForSeoOrganicResult {
    pub rank: i32,
    pub title: String,
    pub url: String,
    pub url_norm: String,
    pub domain_norm: String,
    pub snippet: String,
    pub raw_json: Value,
}

#[derive(Debug, Clone)]
pub struct DataForSeoSerpResponse {
    pub raw_json: Value,
    pub raw_payload_utf8: String,
    pub organic_results: Vec<DataForSeoOrganicResult>,
}

pub struct DataForSeoSerpClient {
    client: Client,
    config: DataForSeoConfig,
}

impl DataForSeoConfig {
    pub fn from_env_with_locale(locale: Option<&str>) -> Option<Self> {
        let login = std::env::var("DATAFORSEO_LOGIN").ok()?;
        let password = std::env::var("DATAFORSEO_PASSWORD").ok()?;
        let endpoint =
            std::env::var("DATAFORSEO_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
        let language_code = std::env::var("DATAFORSEO_LANGUAGE_CODE")
            .ok()
            .or_else(|| locale.and_then(language_from_locale))
            .unwrap_or_else(|| "en".to_string());
        let location_code = std::env::var("DATAFORSEO_LOCATION_CODE")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(2840);
        let depth = std::env::var("DATAFORSEO_DEPTH")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(10)
            .clamp(1, 100);

        Some(Self {
            endpoint,
            login,
            password,
            language_code,
            location_code,
            depth,
        })
    }
}

impl DataForSeoSerpClient {
    pub fn from_config(config: DataForSeoConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest_adapter::new_default_client(45)?,
            config,
        })
    }

    pub async fn google_organic_live_advanced(
        &self,
        keyword: &str,
    ) -> Result<DataForSeoSerpResponse> {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            bail!("DataForSEO keyword is empty");
        }

        let task = json!({
            "keyword": keyword,
            "language_code": self.config.language_code,
            "location_code": self.config.location_code,
            "depth": self.config.depth,
        });

        let raw_payload_utf8 = self
            .client
            .post(&self.config.endpoint)
            .basic_auth(&self.config.login, Some(&self.config.password))
            .json(&vec![task])
            .send()
            .await
            .context("send DataForSEO SERP request")?
            .error_for_status()
            .context("DataForSEO SERP HTTP status")?
            .text()
            .await
            .context("read DataForSEO SERP response")?;
        let raw_json: Value =
            serde_json::from_str(&raw_payload_utf8).context("parse DataForSEO SERP JSON")?;
        let organic_results = parse_organic_results(&raw_json);

        Ok(DataForSeoSerpResponse {
            raw_json,
            raw_payload_utf8,
            organic_results,
        })
    }
}

fn language_from_locale(locale: &str) -> Option<String> {
    let lang = locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if lang.len() == 2 {
        Some(lang)
    } else {
        None
    }
}

fn json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).map(str::trim)
}

fn json_i32(value: &Value, key: &str) -> Option<i32> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
}

fn parse_organic_results(raw: &Value) -> Vec<DataForSeoOrganicResult> {
    let mut out = Vec::new();
    let Some(tasks) = raw.get("tasks").and_then(Value::as_array) else {
        return out;
    };
    for task in tasks {
        let Some(results) = task.get("result").and_then(Value::as_array) else {
            continue;
        };
        for result in results {
            let Some(items) = result.get("items").and_then(Value::as_array) else {
                continue;
            };
            for item in items {
                if json_str(item, "type") != Some("organic") {
                    continue;
                }
                let url = json_str(item, "url").unwrap_or_default();
                if url.is_empty() {
                    continue;
                }
                let rank = json_i32(item, "rank_group")
                    .or_else(|| json_i32(item, "rank_absolute"))
                    .unwrap_or((out.len() + 1) as i32);
                let domain = json_str(item, "domain")
                    .map(str::to_string)
                    .unwrap_or_else(|| domain_norm(url));
                out.push(DataForSeoOrganicResult {
                    rank,
                    title: json_str(item, "title").unwrap_or_default().to_string(),
                    url: url.to_string(),
                    url_norm: url_norm(url),
                    domain_norm: domain_norm(&domain),
                    snippet: json_str(item, "description")
                        .unwrap_or_default()
                        .to_string(),
                    raw_json: item.clone(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dataforseo_organic_items() {
        let raw = json!({
            "tasks": [{
                "result": [{
                    "items": [
                        {
                            "type": "organic",
                            "rank_group": 1,
                            "title": "Visa fees",
                            "url": "https://www.example.com/visa/fees/",
                            "domain": "www.example.com",
                            "description": "Fee details"
                        },
                        {"type": "paid", "url": "https://ads.example/"}
                    ]
                }]
            }]
        });

        let parsed = parse_organic_results(&raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].rank, 1);
        assert_eq!(parsed[0].url_norm, "example.com/visa/fees");
        assert_eq!(parsed[0].domain_norm, "example.com");
        assert_eq!(parsed[0].snippet, "Fee details");
    }
}
