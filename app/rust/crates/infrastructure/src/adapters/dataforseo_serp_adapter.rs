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
    pub source_tier: String,
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

fn configured_endpoint(value: Option<String>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string())
}

impl DataForSeoConfig {
    pub fn from_env_with_locale(locale: Option<&str>) -> Option<Self> {
        let login = std::env::var("DATAFORSEO_LOGIN").ok()?;
        let password = std::env::var("DATAFORSEO_PASSWORD").ok()?;
        let endpoint = configured_endpoint(std::env::var("DATAFORSEO_ENDPOINT").ok());
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
        validate_successful_task_response(&raw_json)?;
        let organic_results = parse_organic_results(&raw_json);

        Ok(DataForSeoSerpResponse {
            raw_json,
            raw_payload_utf8,
            organic_results,
        })
    }
}

fn validate_successful_task_response(response: &Value) -> Result<()> {
    let status_code = response
        .get("status_code")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if status_code != 20000 {
        bail!("DataForSEO SERP response failed status_code={status_code}");
    }
    let tasks = response
        .get("tasks")
        .and_then(Value::as_array)
        .context("DataForSEO SERP response missing tasks")?;
    if tasks.len() != 1 {
        bail!("DataForSEO SERP expected one task, got {}", tasks.len());
    }
    let task_status = tasks[0]
        .get("status_code")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if task_status != 20000 {
        bail!("DataForSEO SERP task failed status_code={task_status}");
    }
    if tasks[0].get("result").and_then(Value::as_array).is_none() {
        bail!("DataForSEO SERP successful task missing results");
    }
    Ok(())
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
                let source_tier = classify_domain_tier(&domain, url);
                out.push(DataForSeoOrganicResult {
                    rank,
                    title: json_str(item, "title").unwrap_or_default().to_string(),
                    url: url.to_string(),
                    url_norm: url_norm(url),
                    domain_norm: domain_norm(&domain),
                    source_tier,
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

fn classify_domain_tier(domain: &str, url: &str) -> String {
    let value = format!(
        "{} {}",
        domain.to_ascii_lowercase(),
        url.to_ascii_lowercase()
    );
    if value.contains(".gov")
        || value.contains(".gob.")
        || value.contains("embassy")
        || value.contains("consulate")
        || value.contains("mfa.")
        || value.contains("mid.")
        || value.contains("exteriores.gob.es")
    {
        "official".to_string()
    } else if value.contains("vfsglobal")
        || value.contains("vfs ")
        || value.contains(" vfs")
        || value.contains("blsspain")
        || value.contains("blsspainglobal")
    {
        "vfs".to_string()
    } else if value.contains("agency") || value.contains("consult") {
        "agency".to_string()
    } else if value.contains("forum")
        || value.contains("reddit.")
        || value.contains("quora.")
        || value.contains("stackexchange")
        || value.contains("community")
    {
        "forum".to_string()
    } else if value.contains("news")
        || value.contains("blog")
        || value.contains("editorial")
        || value.contains("magazine")
        || value.contains("medium.")
    {
        "editorial".to_string()
    } else {
        "low_trust".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_blank_endpoint_uses_default() {
        assert_eq!(configured_endpoint(None), DEFAULT_ENDPOINT);
        assert_eq!(configured_endpoint(Some(String::new())), DEFAULT_ENDPOINT);
        assert_eq!(configured_endpoint(Some("   ".into())), DEFAULT_ENDPOINT);
        assert_eq!(
            configured_endpoint(Some("https://example.test/serp".into())),
            "https://example.test/serp"
        );
    }

    #[test]
    fn provider_task_failure_is_not_a_successful_empty_serp() {
        let failed = json!({
            "status_code": 20000,
            "tasks": [{"status_code": 40501, "status_message": "Invalid Field: location_code", "result": null}]
        });
        assert!(validate_successful_task_response(&failed)
            .unwrap_err()
            .to_string()
            .contains("40501"));
        assert!(parse_organic_results(&failed).is_empty());
    }

    #[test]
    fn provider_top_level_failure_is_rejected() {
        let failed = json!({"status_code": 50000, "tasks": []});
        assert!(validate_successful_task_response(&failed).is_err());
    }

    #[test]
    fn successful_provider_task_accepts_legitimate_empty_results() {
        let empty = json!({"status_code": 20000, "tasks": [{"status_code": 20000, "result": []}]});
        assert!(validate_successful_task_response(&empty).is_ok());
        assert!(parse_organic_results(&empty).is_empty());
    }

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
        assert_eq!(parsed[0].source_tier, "low_trust");
        assert_eq!(parsed[0].snippet, "Fee details");
    }

    #[test]
    fn classifies_domain_tiers() {
        assert_eq!(
            classify_domain_tier("www.mfa.gov.by", "https://www.mfa.gov.by/visa"),
            "official"
        );
        assert_eq!(
            classify_domain_tier("vfsglobal.com", "https://vfsglobal.com/visa"),
            "vfs"
        );
        assert_eq!(
            classify_domain_tier("reddit.com", "https://reddit.com/r/visas"),
            "forum"
        );
    }
}
