//! Sealed DataForSEO Google Organic Standard HTTP boundary for the separate
//! V2 paid owner. This module is deliberately NOT invoked by the V1 fixture
//! Activity. The V2 caller must first obtain an independently issued durable
//! paid-POST claim and never retry an ambiguous POST.
//!
//! Endpoint, transport and credential provenance are operator-owned; none of
//! them are accepted from a siteArchitect operation payload.

use std::{path::Path, time::Duration};

use anyhow::{ensure, Context, Result};
use dataforseo_contracts::{
    validate_discovery_shape_only, DataForSeoGoogleOrganicSearchInputV1,
    DataForSeoGoogleOrganicSearchOutputV1, DataForSeoOrganicHitV1, DataForSeoSourceStatusV1,
    SCHEMA_REVISION,
};
use reqwest::{Client, Url};
use serde_json::{json, Value};

const PROVIDER_ORIGIN: &str = "https://api.dataforseo.com/";
const POST_PATH: &str = "v3/serp/google/organic/task_post";
const GET_PATH: &str = "v3/serp/google/organic/task_get/advanced/";
const MAX_POST_BYTES: usize = 64 * 1024;
const MAX_GET_BYTES: usize = 2 * 1024 * 1024;

/// Never derive or override this URL from a user-defined URL, proxy, recipe,
/// environment endpoint or HTTP redirection.
fn fixed_provider_origin() -> Result<Url> {
    let url = Url::parse(PROVIDER_ORIGIN)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("api.dataforseo.com")
            && url.port().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none(),
        "provider origin must be the pinned DataForSEO HTTPS origin"
    );
    Ok(url)
}

#[derive(Clone)]
pub(super) struct LiveCredentials {
    login: String,
    password: String,
}

impl std::fmt::Debug for LiveCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveCredentials(<redacted>)")
    }
}

impl LiveCredentials {
    /// Operator-mounted 0400/0600 regular file, one `login:password` line.
    /// The user plan cannot supply credentials or the path.
    fn read_from_file(path: &Path) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "DataForSEO credential path must be absolute"
        );
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            meta.file_type().is_file(),
            "credential file must be regular, not symlink"
        );
        ensure!(
            meta.len() > 2 && meta.len() <= 8 * 1024,
            "credential file is empty or oversized"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                meta.permissions().mode() & 0o077 == 0,
                "DataForSEO credential file must be private (0400/0600)"
            );
        }
        let raw = std::fs::read_to_string(path)?;
        let raw = raw.trim_end_matches(|ch| ch == '\r' || ch == '\n');
        let (login, password) = raw
            .split_once(':')
            .context("DataForSEO credential file must contain a login and password")?;
        ensure!(
            !login.is_empty()
                && !password.is_empty()
                && login.len() <= 256
                && password.len() <= 4096
                && login == login.trim()
                && password == password.trim()
                && !raw.chars().any(char::is_control),
            "invalid DataForSEO credential file"
        );
        Ok(Self {
            login: login.to_owned(),
            password: password.to_owned(),
        })
    }

    pub(super) fn from_operator_file_env() -> Result<Self> {
        let filename = std::env::var("DATAFORSEO_PAID_CREDENTIALS_FILE")
            .context("paid DataForSEO credentials require an operator-mounted private file")?;
        Self::read_from_file(Path::new(&filename))
    }
}

#[derive(Debug, Clone)]
pub(super) struct PostedTask {
    pub task_id: String,
    pub raw_bytes: Vec<u8>,
    pub reported_cost_usd_micros: u64,
}

#[derive(Debug, Clone)]
pub(super) struct CompletedTask {
    pub raw_bytes: Vec<u8>,
    pub organic_hits: Vec<DataForSeoOrganicHitV1>,
    pub reported_cost_usd_micros: u64,
}

#[derive(Debug, Clone)]
pub(super) enum TaskRead {
    Pending,
    Completed(CompletedTask),
}

fn valid_task_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !value.starts_with("stub-serp-")
}

/// Exact decimal conversion with **round-up** at USD 1e-6 precision. Never
/// silently truncate a reported cost. Supports JSON's exponent spelling.
fn usd_micros(value: &Value) -> Result<u64> {
    let raw = match value {
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.clone(),
        _ => anyhow::bail!("DataForSEO reported a non-numeric cost"),
    };
    ensure!(!raw.starts_with('-'), "negative provider cost");
    let (mantissa, exponent) = match raw.find(|ch| ch == 'e' || ch == 'E') {
        Some(index) => (&raw[..index], raw[index + 1..].parse::<i32>()?),
        None => (raw.as_str(), 0),
    };
    // `i32::MIN.abs()` would panic; never apply abs() to untrusted JSON.
    ensure!(
        (-256..=15).contains(&exponent),
        "unbounded provider cost exponent"
    );
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    ensure!(
        !whole.is_empty()
            && whole.bytes().all(|byte| byte.is_ascii_digit())
            && fraction.bytes().all(|byte| byte.is_ascii_digit()),
        "malformed provider cost"
    );
    let decimal = format!("{whole}{fraction}");
    let digits = decimal.parse::<u128>()?;
    let decimal_shift = exponent + 6 - i32::try_from(fraction.len())?;
    let micros = if decimal_shift >= 0 {
        ensure!(decimal_shift <= 38, "DataForSEO cost overflow");
        digits
            .checked_mul(10_u128.pow(u32::try_from(decimal_shift)?))
            .context("DataForSEO cost overflow")?
    } else if -decimal_shift > 38 {
        // Any positive value smaller than one USD micro-unit must still
        // reserve one micro-unit, without exponentiation overflow.
        if digits > 0 {
            1
        } else {
            0
        }
    } else {
        let divisor = 10_u128.pow(u32::try_from(-decimal_shift)?);
        digits
            .checked_add(divisor - 1)
            .context("cost rounding overflow")?
            / divisor
    };
    Ok(u64::try_from(micros)?)
}

fn provider_task(bytes: &[u8], cap: usize) -> Result<Value> {
    ensure!(
        bytes.len() <= cap,
        "DataForSEO raw provider response exceeds cap"
    );
    let parsed: Value = serde_json::from_slice(bytes)?;
    ensure!(
        parsed["status_code"].as_i64() == Some(20000),
        "provider envelope rejected"
    );
    let tasks = parsed["tasks"].as_array().context("DataForSEO tasks missing")?;
    ensure!(
        tasks.len() == 1,
        "DataForSEO response is not exactly one task"
    );
    Ok(tasks[0].clone())
}

pub(super) fn parse_post_receipt(
    bytes: &[u8],
    input: &DataForSeoGoogleOrganicSearchInputV1,
) -> Result<PostedTask> {
    let task = provider_task(bytes, MAX_POST_BYTES)?;
    ensure!(
        task["status_code"].as_i64() == Some(20100),
        "DataForSEO task was not created"
    );
    let task_id = task["id"].as_str().context("missing provider task ID")?;
    ensure!(valid_task_id(task_id), "unexpected provider task ID");
    let data = &task["data"];
    ensure!(
        data["keyword"].as_str() == Some(input.keyword.as_str())
            && data["language_code"].as_str() == Some(input.language_code.as_str())
            && data["location_code"].as_i64() == Some(input.location_code)
            && data["depth"].as_u64() == Some(u64::from(input.depth)),
        "DataForSEO provider task identity differs from requested query"
    );
    let cost = usd_micros(&task["cost"])?;
    ensure!(
        cost <= input.max_cost_usd_micros,
        "DataForSEO task cost exceeds grant cap"
    );
    Ok(PostedTask {
        task_id: task_id.to_owned(),
        raw_bytes: bytes.to_vec(),
        reported_cost_usd_micros: cost,
    })
}

fn parse_organic_hits(value: &Value, depth: u32) -> Result<Vec<DataForSeoOrganicHitV1>> {
    let mut hits = Vec::new();
    let mut ranks = std::collections::BTreeSet::new();
    let results = value["result"]
        .as_array()
        .context("missing completed provider result")?;
    for result in results {
        if let Some(items) = result["items"].as_array() {
            for item in items {
                if item["type"].as_str() != Some("organic") {
                    continue;
                }
                let rank = item["rank_group"]
                    .as_u64()
                    .or_else(|| item["rank_absolute"].as_u64())
                    .and_then(|rank| u32::try_from(rank).ok())
                    .context("organic rank missing")?;
                ensure!(
                    rank > 0 && rank <= depth && ranks.insert(rank),
                    "invalid organic rank"
                );
                let title = item["title"].as_str().context("missing organic title")?;
                let url = item["url"].as_str().context("missing organic source URL")?;
                let source = Url::parse(url)?;
                ensure!(
                    source.scheme() == "https"
                        && source.username().is_empty()
                        && source.password().is_none()
                        && source.host_str().is_some()
                        && source.fragment().is_none(),
                    "organic discovery URL must be direct HTTPS"
                );
                let snippet = item["description"].as_str().unwrap_or_default();
                let domain = source.host_str().unwrap_or_default().to_ascii_lowercase();
                ensure!(
                    title.len() <= 4096
                        && url.len() <= 2048
                        && snippet.len() <= 8192
                        && domain.len() <= 256,
                    "organic discovery fields exceed typed bounds"
                );
                hits.push(DataForSeoOrganicHitV1 {
                    rank,
                    title: title.to_owned(),
                    source_url: url.to_owned(),
                    domain_norm: domain,
                    snippet: snippet.to_owned(),
                });
            }
        }
    }
    ensure!(
        hits.len() <= usize::try_from(depth)?,
        "too many organic results"
    );
    Ok(hits)
}

pub(super) fn parse_get_response(
    bytes: &[u8],
    task_id: &str,
    input: &DataForSeoGoogleOrganicSearchInputV1,
) -> Result<TaskRead> {
    ensure!(
        valid_task_id(task_id),
        "invalid paid task id for task_get"
    );
    let task = provider_task(bytes, MAX_GET_BYTES)?;
    ensure!(
        task["id"].as_str() == Some(task_id),
        "DataForSEO GET task ID mismatch"
    );
    let status = task["status_code"]
        .as_i64()
        .context("missing task status")?;
    if status == 40601 || status == 40602 {
        ensure!(
            task["result"].is_null(),
            "queued provider task already has results"
        );
        return Ok(TaskRead::Pending);
    }
    ensure!(
        status == 20000,
        "DataForSEO task_get returned a terminal error"
    );
    let hits = parse_organic_hits(&task, input.depth)?;
    let cost = usd_micros(&task["cost"])?;
    ensure!(
        cost <= input.max_cost_usd_micros,
        "DataForSEO GET cost exceeds cap"
    );
    Ok(TaskRead::Completed(CompletedTask {
        raw_bytes: bytes.to_vec(),
        organic_hits: hits,
        reported_cost_usd_micros: cost,
    }))
}

pub(super) fn validated_output(
    input: &DataForSeoGoogleOrganicSearchInputV1,
    post: &PostedTask,
    get: &CompletedTask,
) -> Result<DataForSeoGoogleOrganicSearchOutputV1> {
    let cost = post
        .reported_cost_usd_micros
        .checked_add(get.reported_cost_usd_micros)
        .context("DataForSEO combined cost overflow")?;
    ensure!(
        cost <= input.max_cost_usd_micros,
        "DataForSEO total cost exceeds grant cap"
    );
    let result = DataForSeoGoogleOrganicSearchOutputV1 {
        schema_revision: SCHEMA_REVISION,
        source_status: DataForSeoSourceStatusV1::SerpDiscoveryOnly,
        provider_task_id: post.task_id.clone(),
        provider_response_hash: blake3::hash(&get.raw_bytes).to_hex().to_string(),
        input_hash: blake3::hash(&serde_json::to_vec(input)?).to_hex().to_string(),
        provider_reported_cost_usd_micros: Some(cost),
        organic_results: get.organic_hits.clone(),
    };
    validate_discovery_shape_only(&result, input)?;
    Ok(result)
}

/// The only future live network client. It must be called strictly after a
/// committed V2 paid ledger claim and a verified one-time operator grant.
pub(super) struct LiveDataForSeoHttp {
    client: Client,
    credentials: LiveCredentials,
    origin: Url,
}

impl LiveDataForSeoHttp {
    pub(super) fn new(credentials: LiveCredentials) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(18))
                .build()?,
            credentials,
            origin: fixed_provider_origin()?,
        })
    }

    /// A paid POST is at-most-once: never call this again when receipt is lost.
    /// The owner caller, not this HTTP primitive, owns durable SQL fencing.
    pub(super) async fn post_once(
        &self,
        input: &DataForSeoGoogleOrganicSearchInputV1,
    ) -> Result<PostedTask> {
        let url = self.origin.join(POST_PATH)?;
        let response = self
            .client
            .post(url)
            .basic_auth(&self.credentials.login, Some(&self.credentials.password))
            .json(&vec![json!({
                "keyword": input.keyword,
                "language_code": input.language_code,
                "location_code": input.location_code,
                "depth": input.depth,
                "priority": 1,
            })])
            .send()
            .await?
            .error_for_status()?;
        let bytes = read_bounded(response, MAX_POST_BYTES).await?;
        parse_post_receipt(&bytes, input)
    }

    /// Safe GET-only continuation after a durable provider task ID exists.
    pub(super) async fn get_posted(
        &self,
        task_id: &str,
        input: &DataForSeoGoogleOrganicSearchInputV1,
    ) -> Result<TaskRead> {
        ensure!(valid_task_id(task_id), "invalid saved provider task ID");
        let url = self.origin.join(&format!("{GET_PATH}{task_id}"))?;
        let response = self
            .client
            .get(url)
            .basic_auth(&self.credentials.login, Some(&self.credentials.password))
            .send()
            .await?
            .error_for_status()?;
        let bytes = read_bounded(response, MAX_GET_BYTES).await?;
        parse_get_response(&bytes, task_id, input)
    }
}

async fn read_bounded(mut response: reqwest::Response, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= cap.saturating_sub(bytes.len()),
            "DataForSEO response exceeds cap"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::RunScope;

    fn query() -> DataForSeoGoogleOrganicSearchInputV1 {
        DataForSeoGoogleOrganicSearchInputV1 {
            schema_revision: SCHEMA_REVISION,
            scope: RunScope {
                tenant_id: "tenant-a".into(),
                run_id: "123e4567-e89b-42d3-a456-426614174000".into(),
            },
            keyword: "custom kitchens".into(),
            language_code: "en".into(),
            location_code: 2840,
            depth: 10,
            max_cost_usd_micros: 10_000,
        }
    }

    fn post() -> Vec<u8> {
        br#"{"status_code":20000,"tasks":[{"status_code":20100,"id":"11151548-1307-0066-0000-aecf8c56ea58","cost":0.0006,"data":{"keyword":"custom kitchens","language_code":"en","location_code":2840,"depth":10}}]}"#
            .to_vec()
    }

    fn get() -> Vec<u8> {
        br#"{"status_code":20000,"tasks":[{"status_code":20000,"id":"11151548-1307-0066-0000-aecf8c56ea58","cost":0,"result":[{"items":[{"type":"organic","rank_group":1,"rank_absolute":3,"title":"Kitchens","url":"https://example.com/kitchens","description":"Sample"}]}]}]}"#
            .to_vec()
    }

    #[test]
    fn valid_paid_post_get_bytes_produce_discovery_only_not_truth() {
        let input = query();
        let posted = parse_post_receipt(&post(), &input).unwrap();
        assert_eq!(posted.reported_cost_usd_micros, 600);
        assert_eq!(posted.raw_bytes, post());
        let read = parse_get_response(&get(), &posted.task_id, &input).unwrap();
        let TaskRead::Completed(complete) = read else {
            panic!("expected complete")
        };
        assert_eq!(complete.organic_hits[0].rank, 1);
        let output = validated_output(&input, &posted, &complete).unwrap();
        assert_eq!(
            output.source_status,
            DataForSeoSourceStatusV1::SerpDiscoveryOnly
        );
        assert_eq!(output.provider_reported_cost_usd_micros, Some(600));
        assert_eq!(
            output.provider_response_hash,
            blake3::hash(&get()).to_hex().to_string()
        );
    }

    #[test]
    fn immutable_task_provenance_and_cost_fail_closed() {
        let input = query();
        for (old, bad) in [
            ("custom kitchens", "different query"),
            ("2840", "2203"),
            ("20100", "40501"),
            ("0.0006", "0.6"),
        ] {
            let modified = String::from_utf8(post()).unwrap().replace(old, bad);
            assert!(parse_post_receipt(modified.as_bytes(), &input).is_err());
        }
        assert!(parse_get_response(&get(), "wrong-task-id", &input).is_err());
        let tampered = String::from_utf8(get())
            .unwrap()
            .replace("https://example.com/", "http://127.0.0.1/");
        assert!(parse_get_response(
            tampered.as_bytes(),
            "11151548-1307-0066-0000-aecf8c56ea58",
            &input
        )
        .is_err());
    }

    #[test]
    fn pending_is_explicit_and_cannot_be_claimed_as_result() {
        let queued = br#"{"status_code":20000,"tasks":[{"status_code":40602,"id":"11151548-1307-0066-0000-aecf8c56ea58","result":null}]}"#;
        assert!(matches!(
            parse_get_response(queued, "11151548-1307-0066-0000-aecf8c56ea58", &query())
                .unwrap(),
            TaskRead::Pending
        ));
        let broken = br#"{"status_code":20000,"tasks":[{"status_code":40401,"id":"11151548-1307-0066-0000-aecf8c56ea58","result":null}]}"#;
        assert!(
            parse_get_response(broken, "11151548-1307-0066-0000-aecf8c56ea58", &query())
                .is_err()
        );
    }

    #[test]
    fn cost_conversion_rounds_up_and_denies_overflow() {
        for (sample, expected) in [
            ("0.0000001", 1_u64),
            ("0.0006", 600),
            ("1e-6", 1),
            ("0", 0),
            ("0.00000000000000000000001", 1),
        ] {
            let value: Value = serde_json::from_str(sample).unwrap();
            assert_eq!(usd_micros(&value).unwrap(), expected);
        }
        for sample in ["-1", "1e30", "1e-2147483648"] {
            let value = Value::String(sample.to_owned());
            assert!(usd_micros(&value).is_err());
        }
    }

    #[test]
    fn provider_origin_is_pinned_and_credentials_debug_redacted() {
        let url = fixed_provider_origin().unwrap();
        assert_eq!(url.as_str(), PROVIDER_ORIGIN);
        let credentials = LiveCredentials {
            login: "secret-login".into(),
            password: "secret-password".into(),
        };
        assert!(!format!("{credentials:?}").contains("secret"));
        let client = LiveDataForSeoHttp::new(credentials).unwrap();
        assert_eq!(client.origin, url);
    }

    #[cfg(unix)]
    #[test]
    fn private_operator_credential_file_required_without_secret_leakage() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = std::env::temp_dir().join(format!(
            "dataforseo-private-credential-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let secret = root.join("provider-login");
        std::fs::write(&secret, "login-example:password-example\n").unwrap();

        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let verified = LiveCredentials::read_from_file(&secret).unwrap();
        assert_eq!(verified.login, "login-example");
        assert_eq!(verified.password, "password-example");

        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
        let world_readable = LiveCredentials::read_from_file(&secret)
            .expect_err("world-readable provider credentials must be rejected");
        assert!(!world_readable.to_string().contains("password-example"));
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();

        let symlink_path = root.join("linked-provider-login");
        symlink(&secret, &symlink_path).unwrap();
        assert!(LiveCredentials::read_from_file(&symlink_path).is_err());
        assert!(LiveCredentials::read_from_file(Path::new("relative-secret")).is_err());

        std::fs::remove_file(&symlink_path).unwrap();
        std::fs::remove_file(&secret).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }
}
