use contracts::generated::alegria::temporal::v1::{
    ContentGapState, KeywordClusterState, OpportunityBuildInputPayload,
    OpportunityBuildOutputPayload,
};

use crate::seo_step_support::{artifact_key, scope_signature};

pub fn execute(input: &OpportunityBuildInputPayload) -> OpportunityBuildOutputPayload {
    let fallback_scope = scope_signature(input.scope.as_ref());
    let mut keyword_clusters = Vec::new();
    let mut content_gaps = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for pattern in &input.serp_patterns {
        let scope = if pattern.scope_signature.is_empty() {
            fallback_scope.clone()
        } else {
            pattern.scope_signature.clone()
        };
        let seed_keyword = pattern.query.trim().to_ascii_lowercase();
        if seed_keyword.is_empty() || !seen.insert((scope.clone(), seed_keyword.clone())) {
            continue;
        }
        let cluster_key = artifact_key(
            "keyword_cluster",
            &[
                &scope,
                &seed_keyword,
                &pattern.dominant_intent,
                "seo_cluster@1",
            ],
        );
        keyword_clusters.push(KeywordClusterState {
            cluster_key: cluster_key.clone(),
            scope_signature: scope.clone(),
            seed_keyword: seed_keyword.clone(),
            dominant_intent: if pattern.dominant_intent.is_empty() {
                "informational".to_string()
            } else {
                pattern.dominant_intent.clone()
            },
            status: "candidate".to_string(),
            cluster_version: 1,
        });
        if pattern.reliability_score < 0.5 {
            content_gaps.push(ContentGapState {
                content_gap_key: artifact_key(
                    "content_gap",
                    &[
                        &scope,
                        &seed_keyword,
                        "low_reliability_serp",
                        "seo_content_gap@1",
                    ],
                ),
                scope_signature: scope,
                page_node_key: String::new(),
                missing_topic: seed_keyword,
                severity: "medium".to_string(),
                status: "open".to_string(),
            });
        }
    }

    OpportunityBuildOutputPayload {
        keyword_clusters,
        content_gaps,
    }
}
