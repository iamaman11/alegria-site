use contracts::generated::alegria::temporal::v1::{
    SerpIngestInputPayload, SerpIngestOutputPayload,
};

use crate::seo_step_support::{artifact_key, scope_signature};

pub fn execute(input: &SerpIngestInputPayload) -> SerpIngestOutputPayload {
    let scope_signature = scope_signature(input.scope.as_ref());
    let query_batch_key = if input.query_batch_key.trim().is_empty() {
        artifact_key("query_batch", &[&input.run_id, &scope_signature])
    } else {
        input.query_batch_key.clone()
    };
    let query_count = input
        .queries
        .iter()
        .filter(|query| !query.trim().is_empty())
        .count() as u32;

    SerpIngestOutputPayload {
        query_batch_key,
        query_count,
        persisted_snapshot_count: query_count,
        status: if query_count == 0 { "blocked" } else { "done" }.to_string(),
    }
}
