//! Build editorial blocks from authoritative, admissible PostgreSQL Truth.
//! The reader is replaceable; the role projection and rendering are pure Rust.

use std::collections::BTreeMap;

use primitives::errors::DomainError;
use runtime_models::{RuleParams, RuleRoleType};
use seo_domain::writer::{self, CitationRule, FaqItem, RuleParamsView, RuleRole};
use seo_ports::{
    knowledge::{
        CanonicalVectorCandidate, CanonicalVectorLookup, VerifiedRule, VerifiedRulesQuery,
    },
    CanonicalVectorSearchPort, VerifiedRulesReader,
};
use seo_steps::canonical_mapping_step::{MappingResult, MatchingStage};

/// Resolve symbolic terms against non-authoritative vector suggestions. Only
/// this mapping result changes; verified rules remain PostgreSQL-owned.
pub async fn resolve_canonical_vector_mappings<S: CanonicalVectorSearchPort + ?Sized>(
    search: &S,
    mappings: Vec<MappingResult>,
) -> Result<Vec<MappingResult>, DomainError> {
    let mut resolved = Vec::with_capacity(mappings.len());
    for mapping in mappings {
        if mapping.matching_stage != MatchingStage::VectorQdrant
            || mapping.raw_text.trim().is_empty()
        {
            resolved.push(mapping);
            continue;
        }
        let updated = match search.search_canonical_concepts(&mapping.raw_text).await? {
            CanonicalVectorLookup::Unavailable => mapping,
            CanonicalVectorLookup::Candidates(candidates) if candidates.is_empty() => {
                unresolved_canonical_mapping(&mapping, "qdrant_no_candidates")
            }
            CanonicalVectorLookup::Candidates(candidates) => {
                resolve_canonical_candidates(&mapping, &candidates)
            }
        };
        resolved.push(updated);
    }
    Ok(resolved)
}

fn unresolved_canonical_mapping(mapping: &MappingResult, method: &str) -> MappingResult {
    MappingResult {
        raw_text: mapping.raw_text.clone(),
        canonical_key: None,
        mapping_type: "new_candidate".to_string(),
        match_method: method.to_string(),
        matching_stage: MatchingStage::VectorQdrant,
        qdrant_score: None,
        confidence: 0.0,
        needs_hitl: true,
    }
}

fn resolve_canonical_candidates(
    mapping: &MappingResult,
    candidates: &[CanonicalVectorCandidate],
) -> MappingResult {
    let top = &candidates[0];
    // Provider scores and candidate identities are untrusted. An invalid hit
    // cannot produce an automatic match even if its reported score is high.
    let Some(top_key) = top
        .canonical_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
    else {
        return unresolved_canonical_mapping(mapping, "qdrant_invalid_candidate");
    };
    if !top.score.is_finite()
        || !(0.0..=1.0).contains(&top.score)
        || candidates
            .iter()
            .skip(1)
            .any(|candidate| !candidate.score.is_finite())
    {
        return unresolved_canonical_mapping(mapping, "qdrant_invalid_score");
    }
    let margin = candidates
        .get(1)
        .map_or(top.score, |second| top.score - second.score);
    let (mapping_type, needs_hitl, canonical_key) = if top.score >= 0.88 && margin >= 0.03 {
        ("auto_map", false, Some(top_key.to_string()))
    } else if top.score >= 0.75 {
        ("review", true, Some(top_key.to_string()))
    } else {
        ("new_candidate", true, None)
    };
    MappingResult {
        raw_text: mapping.raw_text.clone(),
        canonical_key,
        mapping_type: mapping_type.to_string(),
        match_method: "qdrant_retrieval".to_string(),
        matching_stage: MatchingStage::VectorQdrant,
        qdrant_score: Some(top.score),
        confidence: top.score,
        needs_hitl,
    }
}

pub async fn generate_verified_blocks<R: VerifiedRulesReader>(
    reader: &R,
    query: &VerifiedRulesQuery,
) -> Result<BTreeMap<String, String>, DomainError> {
    if query.context_key.trim().is_empty() {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "knowledge.read_verified_rules stage=query run_id={} context_key is empty",
                query.run_id
            ),
        });
    }

    let context = reader.read_verified_rules(query).await?.ok_or_else(|| {
        DomainError::ValidationFailure {
            message: format!(
                "knowledge.read_verified_rules stage=active_context run_id={} context_key={} is not active",
                query.run_id, query.context_key
            ),
        }
    })?;
    if context.context_key != query.context_key {
        return Err(DomainError::ContractViolation {
            message: format!(
                "knowledge.read_verified_rules stage=result_identity run_id={} requested={} returned={}",
                query.run_id, query.context_key, context.context_key
            ),
        });
    }
    if context.rules.is_empty() {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "knowledge.read_verified_rules stage=verified_support run_id={} context_key={} has no admissible verified rules",
                query.run_id, query.context_key
            ),
        });
    }

    let rules: Vec<_> = context
        .rules
        .into_iter()
        .map(|rule| project_writer_rule(rule, &query.run_id))
        .collect::<Result<Vec<_>, DomainError>>()?;
    let faq_items: Vec<FaqItem> = Vec::new();
    Ok(writer::plan_blocks(&rules, &faq_items)
        .into_iter()
        .map(|block_key| {
            let html = writer::render_block(&block_key, &rules, &faq_items);
            (block_key, html)
        })
        .collect())
}

fn project_writer_rule(rule: VerifiedRule, run_id: &str) -> Result<CitationRule, DomainError> {
    let invalid_params = |field: &str| {
        DomainError::ContractViolation {
        message: format!(
            "knowledge.read_verified_rules stage=required_params run_id={run_id} rule_id={} invalid {field}",
            rule.rule_instance_id
        ),
    }
    };
    if (matches!(rule.role, RuleRoleType::FeeItem | RuleRoleType::MustPay)
        || matches!(rule.rule_type_key.as_str(), "fee" | "consular_fee"))
        && !matches!(&rule.params, RuleParams::Fee { .. })
    {
        return Err(invalid_params("fee"));
    }
    if matches!(
        rule.role,
        RuleRoleType::TimelineItem | RuleRoleType::Timeline
    ) && !matches!(&rule.params, RuleParams::Timeline { .. })
    {
        return Err(invalid_params("timeline"));
    }
    match &rule.params {
        RuleParams::Fee {
            amount, currency, ..
        } if !amount.is_finite() || currency.trim().is_empty() => {
            return Err(invalid_params("fee amount/currency"));
        }
        RuleParams::Timeline { days, .. } if *days < 0 => {
            return Err(invalid_params("timeline days"));
        }
        _ => {}
    }
    let mut role = match rule.role {
        RuleRoleType::DocumentRequired | RuleRoleType::MustProvide => RuleRole::DocumentRequired,
        RuleRoleType::FormRequired => RuleRole::FormRequired,
        RuleRoleType::FeeItem | RuleRoleType::MustPay => RuleRole::FeeItem,
        RuleRoleType::TimelineItem | RuleRoleType::Timeline => RuleRole::TimelineItem,
        _ => RuleRole::Generic,
    };
    let params = match rule.params {
        RuleParams::Document {
            severity,
            subtype,
            notarization_required,
            ..
        } => {
            role = RuleRole::DocumentRequired;
            RuleParamsView::Document {
                severity,
                subtype: subtype.filter(|value| !value.is_empty()),
                notarization_required,
            }
        }
        RuleParams::Fee {
            amount, currency, ..
        } => {
            role = RuleRole::FeeItem;
            RuleParamsView::Fee { amount, currency }
        }
        RuleParams::Timeline { days, .. } => {
            role = RuleRole::TimelineItem;
            RuleParamsView::Timeline { days }
        }
        RuleParams::None => RuleParamsView::None,
        _ => {
            role = RuleRole::Generic;
            RuleParamsView::None
        }
    };

    Ok(CitationRule {
        rule_instance_id: rule.rule_instance_id,
        rule_type_key: rule.rule_type_key,
        role,
        params,
        status: "verified".to_string(),
        effective_from: rule.effective_from,
        effective_to: rule.effective_to,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use primitives::errors::ErrorClass;
    use seo_ports::knowledge::VerifiedRulesView;

    struct FakeCanonicalSearch {
        lookup: CanonicalVectorLookup,
    }

    #[async_trait]
    impl CanonicalVectorSearchPort for FakeCanonicalSearch {
        async fn search_canonical_concepts(
            &self,
            _raw_text: &str,
        ) -> Result<CanonicalVectorLookup, DomainError> {
            Ok(self.lookup.clone())
        }
    }

    struct FailIfCalled;

    #[async_trait]
    impl CanonicalVectorSearchPort for FailIfCalled {
        async fn search_canonical_concepts(
            &self,
            _raw_text: &str,
        ) -> Result<CanonicalVectorLookup, DomainError> {
            panic!("non-vector mapping must not invoke an external provider")
        }
    }

    struct BrokenProvider;

    #[async_trait]
    impl CanonicalVectorSearchPort for BrokenProvider {
        async fn search_canonical_concepts(
            &self,
            _raw_text: &str,
        ) -> Result<CanonicalVectorLookup, DomainError> {
            Err(DomainError::InfraUnavailable {
                message: "simulated qdrant outage".into(),
            })
        }
    }

    fn symbolic_mapping() -> MappingResult {
        MappingResult {
            raw_text: "unseen authority term".into(),
            canonical_key: Some("candidate:unseen_authority_term".into()),
            mapping_type: "vector_required".into(),
            match_method: "vector_qdrant_required".into(),
            matching_stage: MatchingStage::VectorQdrant,
            qdrant_score: None,
            confidence: 0.0,
            needs_hitl: true,
        }
    }

    fn hit(key: Option<&str>, score: f32) -> CanonicalVectorCandidate {
        CanonicalVectorCandidate {
            canonical_key: key.map(str::to_owned),
            score,
        }
    }

    async fn resolve(lookup: CanonicalVectorLookup) -> MappingResult {
        resolve_canonical_vector_mappings(&FakeCanonicalSearch { lookup }, vec![symbolic_mapping()])
            .await
            .unwrap()
            .remove(0)
    }

    #[tokio::test]
    async fn empty_search_requires_hitl_and_cannot_promote_symbolic_candidate() {
        let mapping = resolve(CanonicalVectorLookup::Candidates(vec![])).await;
        assert!(mapping.canonical_key.is_none());
        assert!(mapping.needs_hitl);
        assert_eq!(mapping.match_method, "qdrant_no_candidates");
        assert_eq!(mapping.confidence, 0.0);
        assert_eq!(mapping.matching_stage, MatchingStage::VectorQdrant);
    }

    #[tokio::test]
    async fn unavailable_optional_provider_preserves_unresolved_hitl_gate() {
        let mapping = resolve(CanonicalVectorLookup::Unavailable).await;
        assert_eq!(mapping.canonical_key, symbolic_mapping().canonical_key);
        assert!(mapping.needs_hitl);
        assert_eq!(mapping.mapping_type, "vector_required");
    }

    #[tokio::test]
    async fn high_confidence_requires_identity_and_margin_for_automatic_mapping() {
        let confident = resolve(CanonicalVectorLookup::Candidates(vec![
            hit(Some("canonical:passport"), 0.94),
            hit(Some("canonical:other"), 0.80),
        ]))
        .await;
        assert!(!confident.needs_hitl);
        assert_eq!(confident.mapping_type, "auto_map");
        assert_eq!(
            confident.canonical_key.as_deref(),
            Some("canonical:passport")
        );

        let ambiguous = resolve(CanonicalVectorLookup::Candidates(vec![
            hit(Some("canonical:passport"), 0.94),
            hit(Some("canonical:other"), 0.93),
        ]))
        .await;
        assert!(ambiguous.needs_hitl);
        assert_eq!(ambiguous.mapping_type, "review");

        let weak = resolve(CanonicalVectorLookup::Candidates(vec![hit(
            Some("canonical:passport"),
            0.74,
        )]))
        .await;
        assert!(weak.needs_hitl);
        assert!(weak.canonical_key.is_none());
    }

    #[tokio::test]
    async fn invalid_vector_identity_or_scores_never_auto_map() {
        for candidates in [
            vec![hit(None, 0.99)],
            vec![hit(Some("  "), 0.99)],
            vec![hit(Some("canonical:passport"), f32::NAN)],
            vec![hit(Some("canonical:passport"), 1.5)],
            vec![hit(Some("canonical:passport"), 0.99), hit(None, f32::NAN)],
        ] {
            let mapping = resolve(CanonicalVectorLookup::Candidates(candidates)).await;
            assert!(mapping.needs_hitl);
            assert!(mapping.canonical_key.is_none());
            assert_eq!(mapping.confidence, 0.0);
        }
    }

    #[tokio::test]
    async fn symbolic_search_does_not_mask_typed_provider_failures() {
        let err = resolve_canonical_vector_mappings(&BrokenProvider, vec![symbolic_mapping()])
            .await
            .unwrap_err();
        assert_eq!(err.class(), ErrorClass::InfraUnavailable);
        assert!(err.to_string().contains("simulated qdrant outage"));
    }

    #[tokio::test]
    async fn symbolic_retrieval_never_runs_for_exact_alias_mapping() {
        let fixed = MappingResult {
            matching_stage: MatchingStage::ExactAlias,
            needs_hitl: false,
            mapping_type: "alias".into(),
            ..symbolic_mapping()
        };
        let unchanged = resolve_canonical_vector_mappings(&FailIfCalled, vec![fixed.clone()])
            .await
            .unwrap();
        assert_eq!(unchanged[0].canonical_key, fixed.canonical_key);
        assert!(!unchanged[0].needs_hitl);
    }

    struct FakeReader {
        context: Option<VerifiedRulesView>,
    }

    #[async_trait]
    impl VerifiedRulesReader for FakeReader {
        async fn read_verified_rules(
            &self,
            _query: &VerifiedRulesQuery,
        ) -> Result<Option<VerifiedRulesView>, DomainError> {
            Ok(self.context.clone())
        }
    }

    fn query() -> VerifiedRulesQuery {
        VerifiedRulesQuery {
            run_id: "test-run".to_string(),
            context_key: "test-context".to_string(),
        }
    }

    fn rule(role: RuleRoleType, params: RuleParams, key: &str) -> VerifiedRule {
        VerifiedRule {
            rule_instance_id: key.to_string(),
            rule_type_key: key.to_string(),
            role,
            params,
            effective_from: String::new(),
            effective_to: String::new(),
            source_key: Some("government-source".to_string()),
        }
    }

    #[tokio::test]
    async fn verified_rules_render_without_postgres_or_vector_search() {
        let reader = FakeReader {
            context: Some(VerifiedRulesView {
                context_key: "test-context".to_string(),
                rules: vec![
                    rule(
                        RuleRoleType::MustPay,
                        RuleParams::Fee {
                            amount: 35.0,
                            currency: "EUR".into(),
                            severity: String::new(),
                            channel: None,
                            conditions_key: String::new(),
                        },
                        "consular_fee",
                    ),
                    rule(
                        RuleRoleType::DocumentRequired,
                        RuleParams::Document {
                            severity: "mandatory".into(),
                            subtype: Some("passport".into()),
                            notarization_required: true,
                            translation_required: false,
                            accepts_alternatives: false,
                            conditions_key: String::new(),
                        },
                        "document_required",
                    ),
                    rule(
                        RuleRoleType::Timeline,
                        RuleParams::Timeline {
                            days: 14,
                            subtype: None,
                            severity: String::new(),
                            conditions_key: String::new(),
                        },
                        "timeline_item",
                    ),
                ],
            }),
        };
        let blocks = generate_verified_blocks(&reader, &query()).await.unwrap();
        assert!(blocks["fees_table"].contains("35"));
        assert!(blocks["documents_list"].contains("passport"));
        assert!(blocks["timeline"].contains("14"));
    }

    #[tokio::test]
    async fn rejects_inactive_empty_and_mismatched_contexts_without_inventing_truth() {
        for (context, expected_class, stage) in [
            (None, ErrorClass::ValidationFailure, "active_context"),
            (
                Some(VerifiedRulesView {
                    context_key: "test-context".into(),
                    rules: vec![],
                }),
                ErrorClass::ValidationFailure,
                "verified_support",
            ),
            (
                Some(VerifiedRulesView {
                    context_key: "other-context".into(),
                    rules: vec![rule(RuleRoleType::FeeItem, RuleParams::None, "fee")],
                }),
                ErrorClass::ContractViolation,
                "result_identity",
            ),
        ] {
            let error = generate_verified_blocks(&FakeReader { context }, &query())
                .await
                .unwrap_err();
            assert_eq!(error.class(), expected_class);
            assert!(error.to_string().contains(stage));
            assert!(error.to_string().contains("test-run"));
        }
    }

    #[tokio::test]
    async fn incomplete_verified_numeric_rules_fail_closed_with_typed_error() {
        for (role, params, key) in [
            (
                RuleRoleType::FeeItem,
                RuleParams::None,
                "fee_without_params",
            ),
            (RuleRoleType::Allows, RuleParams::None, "consular_fee"),
            (
                RuleRoleType::MustPay,
                RuleParams::Fee {
                    amount: 35.0,
                    currency: String::new(),
                    severity: String::new(),
                    channel: None,
                    conditions_key: String::new(),
                },
                "fee_without_currency",
            ),
            (
                RuleRoleType::TimelineItem,
                RuleParams::None,
                "timeline_without_days",
            ),
        ] {
            let reader = FakeReader {
                context: Some(VerifiedRulesView {
                    context_key: "test-context".into(),
                    rules: vec![rule(role, params, key)],
                }),
            };
            let error = generate_verified_blocks(&reader, &query())
                .await
                .unwrap_err();
            assert_eq!(error.class(), ErrorClass::ContractViolation);
            assert!(error.to_string().contains("stage=required_params"));
            assert!(error.to_string().contains("run_id=test-run"));
            assert!(error.to_string().contains(key));
        }
    }
}
