use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(feature = "e2e")]
pub mod containers;
pub mod env_overrides;
pub mod stub_servers;

#[derive(Debug)]
pub struct IntegrationCtx {
    pub dataforseo: stub_servers::StubServerHandle,
    pub openai: stub_servers::StubServerHandle,
}

impl IntegrationCtx {
    pub async fn start(dataforseo_payload: Value, openai_payload: Value) -> Result<Self> {
        let dataforseo = stub_servers::spawn_json_stub(
            "/v3/serp/google/organic/live/advanced",
            dataforseo_payload,
        )
        .await
        .context("start dataforseo stub failed")?;
        let openai = stub_servers::spawn_json_stub("/v1/chat/completions", openai_payload)
            .await
            .context("start openai stub failed")?;
        Ok(Self { dataforseo, openai })
    }

    pub fn dataforseo_env(&self) -> BTreeMap<String, String> {
        env_overrides::btree_env([
            (
                "DATAFORSEO_ENDPOINT".to_string(),
                format!(
                    "{}/v3/serp/google/organic/live/advanced",
                    self.dataforseo.base_url
                ),
            ),
            ("DATAFORSEO_LOGIN".to_string(), "stub-login".to_string()),
            (
                "DATAFORSEO_PASSWORD".to_string(),
                "stub-password".to_string(),
            ),
            ("DATAFORSEO_LANGUAGE_CODE".to_string(), "en".to_string()),
            ("DATAFORSEO_LOCATION_CODE".to_string(), "2840".to_string()),
            ("DATAFORSEO_DEPTH".to_string(), "10".to_string()),
        ])
    }

    pub fn local_editorial_env(&self) -> BTreeMap<String, String> {
        env_overrides::btree_env([
            (
                "SEO_LLM_PROVIDER".to_string(),
                "local_compatible".to_string(),
            ),
            (
                "SEO_LLM_LOCAL_ENDPOINT".to_string(),
                format!("{}/v1/chat/completions", self.openai.base_url),
            ),
            (
                "SEO_LLM_LOCAL_MODEL".to_string(),
                "stub-editorial-model".to_string(),
            ),
        ])
    }

    pub fn provider_env(&self) -> BTreeMap<String, String> {
        let mut vars = self.dataforseo_env();
        vars.extend(self.local_editorial_env());
        vars
    }

    pub fn apply_provider_env(&self) -> env_overrides::EnvOverrideGuard {
        env_overrides::EnvOverrideGuard::apply(self.provider_env())
    }
}

#[cfg(feature = "e2e")]
pub use containers::{InfraHarness, Neo4jHarness, PostgresHarness, QdrantHarness, TemporalHarness};

#[cfg(test)]
#[allow(unused, warnings)]
mod tests {
    include!("tests/preamble.rs");
    include!("tests/truth_certification_helpers.rs");
    include!("tests/synthetic_runtime_helpers.rs");
    include!("tests/provider_runtime_surface_tests.rs");
    include!("tests/truth_runtime_tests.rs");
    include!("tests/scenario_parity_tests.rs");
}
