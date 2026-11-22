//! Build editorial blocks from authoritative, admissible PostgreSQL Truth.
//! The reader is replaceable; the role projection and rendering are pure Rust.

use std::collections::BTreeMap;

use primitives::errors::DomainError;
use runtime_models::{RuleParams, RuleRoleType};
use seo_domain::writer::{self, CitationRule, FaqItem, RuleParamsView, RuleRole};
use seo_ports::{knowledge::VerifiedRule, knowledge::VerifiedRulesQuery, VerifiedRulesReader};

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

    let rules: Vec<_> = context.rules.into_iter().map(project_writer_rule).collect();
    let faq_items: Vec<FaqItem> = Vec::new();
    Ok(writer::plan_blocks(&rules, &faq_items)
        .into_iter()
        .map(|block_key| {
            let html = writer::render_block(&block_key, &rules, &faq_items);
            (block_key, html)
        })
        .collect())
}

fn project_writer_rule(rule: VerifiedRule) -> CitationRule {
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
            RuleParamsView::Fee {
                amount,
                currency: if currency.is_empty() {
                    "EUR".to_string()
                } else {
                    currency
                },
            }
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

    CitationRule {
        rule_instance_id: rule.rule_instance_id,
        rule_type_key: rule.rule_type_key,
        role,
        params,
        status: "verified".to_string(),
        effective_from: rule.effective_from,
        effective_to: rule.effective_to,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use primitives::errors::ErrorClass;
    use seo_ports::knowledge::VerifiedRulesView;

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
}
