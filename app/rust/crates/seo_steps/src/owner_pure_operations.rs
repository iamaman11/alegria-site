//! Independent, versioned source-owner contracts for pure SEO decisions.
//!
//! These owner DTOs intentionally do not replace permissive historical
//! Temporal V1 protobuf payloads. They are strict, directly schema-generatable
//! boundaries for a separately deployed owner Child/ValidateOnly adapter.
//! Registration here never creates a route, grants source authority, or starts
//! a provider effect. A new owner release does not require editing the DAG.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::error::Error;
use std::fmt;

use contracts::generated::alegria::temporal::v1::{
    ContentGapState, GraphPlanningContextState, GraphPlanningCoverageSignalState,
    GraphPlanningTopicSignalState, GraphPlanningTripleSignalState, KeywordClusterState,
    OpportunityBuildInputPayload, PageNodeState, SeoScopePayload, SerpNormalizeInputPayload,
    SerpPatternState,
};
use operation_protocol::{
    ArtifactSchema, ContractCatalog, IdempotencyPolicy, Operation, OperationContract, OperationId,
    OperationOwnerId, OperationRef, PlanError, PortId,
};
use runtime_models::RuleRoleType;
use schemars::JsonSchema;
use seo_domain::extraction::entity_spans::EntityMention;
use serde::{Deserialize, Serialize};

use crate::{opportunity_build_step, procedural_extraction_step, serp_normalize_step};

/// A real pure evaluator; its Failure is a type owned by this source crate.
/// The Temporal adapter must serialize a typed failure, never treat errors as
/// an empty successful result or retry a provider operation behind this trait.
pub trait OwnerPureOperation: Operation {
    fn evaluate(input: &Self::Input) -> Result<Self::Output, Self::Failure>;
}

fn pure_contract(id: &str, input: &str, output: &str) -> OperationContract {
    OperationContract {
        reference: OperationRef {
            id: OperationId::from(id),
            revision: 1,
        },
        owner: OperationOwnerId::from("seo_steps"),
        inputs: BTreeMap::from([(
            PortId::from("input"),
            ArtifactSchema {
                id: input.into(),
                revision: 1,
            },
        )]),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            ArtifactSchema {
                id: output.into(),
                revision: 1,
            },
        )]),
        invalidates: BTreeSet::new(),
        effects: BTreeSet::new(),
        idempotency: IdempotencyPolicy::Pure,
    }
}

/// The source owner's explicit, versioned contract list; it is not a
/// separately installed trusted route catalog or worker readiness proof.
pub fn owner_pure_contracts() -> Result<ContractCatalog, PlanError> {
    let mut catalog = ContractCatalog::default();
    catalog.register::<ExtractProceduralRules>()?;
    catalog.register::<NormalizeSerpQueries>()?;
    catalog.register::<BuildSeoOpportunities>()?;
    Ok(catalog)
}

// EX-04: only the semantic rule decision. The section read, durable journal,
// and any authorized Truth write are separate effect owners.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProceduralRulesInputV1 {
    pub section_id: String,
    pub raw_text: String,
    pub mentions: Vec<StrictEntityMentionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrictEntityMentionV1 {
    pub raw_text: String,
    pub entity_type: String,
    pub has_numeric: bool,
    pub is_central: bool,
    pub confidence: f32,
}

impl From<&StrictEntityMentionV1> for EntityMention {
    fn from(mention: &StrictEntityMentionV1) -> Self {
        Self {
            raw_text: mention.raw_text.clone(),
            entity_type: mention.entity_type.clone(),
            has_numeric: mention.has_numeric,
            is_central: mention.is_central,
            confidence: mention.confidence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProceduralRuleV1 {
    pub rule_key: String,
    pub role_type: ProceduralRoleV1,
    pub numeric_tokens: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProceduralRoleV1 {
    MustProvide,
    MustPay,
    MustSatisfy,
    Allows,
    Forbids,
    Timeline,
    DocumentRequired,
    EligibilityRule,
    FeeItem,
    TimelineItem,
    WhereToApply,
    AppointmentRule,
    FormRequired,
    Step,
}

impl From<RuleRoleType> for ProceduralRoleV1 {
    fn from(role: RuleRoleType) -> Self {
        match role {
            RuleRoleType::MustProvide => Self::MustProvide,
            RuleRoleType::MustPay => Self::MustPay,
            RuleRoleType::MustSatisfy => Self::MustSatisfy,
            RuleRoleType::Allows => Self::Allows,
            RuleRoleType::Forbids => Self::Forbids,
            RuleRoleType::Timeline => Self::Timeline,
            RuleRoleType::DocumentRequired => Self::DocumentRequired,
            RuleRoleType::EligibilityRule => Self::EligibilityRule,
            RuleRoleType::FeeItem => Self::FeeItem,
            RuleRoleType::TimelineItem => Self::TimelineItem,
            RuleRoleType::WhereToApply => Self::WhereToApply,
            RuleRoleType::AppointmentRule => Self::AppointmentRule,
            RuleRoleType::FormRequired => Self::FormRequired,
            RuleRoleType::Step => Self::Step,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProceduralRulesOutputV1 {
    pub rules: Vec<ProceduralRuleV1>,
}

pub struct ExtractProceduralRules;

impl Operation for ExtractProceduralRules {
    type Input = ProceduralRulesInputV1;
    type Output = ProceduralRulesOutputV1;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "seo_steps.extract_procedural_rules",
            "seo_steps.procedural_rules_input",
            "seo_steps.procedural_rules_output",
        )
    }
}

impl OwnerPureOperation for ExtractProceduralRules {
    fn evaluate(input: &Self::Input) -> Result<Self::Output, Self::Failure> {
        let result = procedural_extraction_step::execute(
            &procedural_extraction_step::ProceduralExtractionInput {
                section_id: input.section_id.clone(),
                raw_text: input.raw_text.clone(),
                mentions: input.mentions.iter().map(EntityMention::from).collect(),
            },
        );
        Ok(ProceduralRulesOutputV1 {
            rules: result
                .rules
                .into_iter()
                .map(|rule| ProceduralRuleV1 {
                    rule_key: rule.rule_key,
                    role_type: rule.role_type.into(),
                    numeric_tokens: rule.numeric_tokens,
                    confidence: rule.confidence,
                })
                .collect(),
        })
    }
}

// PL-02: SERP query normalization is an independent pure algorithm. It does
// not read the Google API or infer that a SERP result was fetched or verified.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningScopeV1 {
    pub market: String,
    pub locale: String,
    pub country_code: String,
    pub visa_type: String,
    pub applicant_profile: String,
    #[serde(default)]
    pub raw_scope_tuple: Option<String>,
    #[serde(default)]
    pub scope_signature: Option<String>,
}

impl From<&PlanningScopeV1> for SeoScopePayload {
    fn from(scope: &PlanningScopeV1) -> Self {
        Self {
            market: scope.market.clone(),
            locale: scope.locale.clone(),
            country_code: scope.country_code.clone(),
            visa_type: scope.visa_type.clone(),
            applicant_profile: scope.applicant_profile.clone(),
            raw_scope_tuple: scope.raw_scope_tuple.clone().unwrap_or_default(),
            scope_signature: scope.scope_signature.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SerpQueryNormalizeInputV1 {
    pub run_id: String,
    #[serde(default)]
    pub query_batch_key: Option<String>,
    #[serde(default)]
    pub scope: Option<PlanningScopeV1>,
    pub queries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizedSerpPatternV1 {
    pub serp_pattern_key: String,
    pub query_batch_key: String,
    pub scope_signature: String,
    pub query: String,
    pub pattern_type: String,
    pub dominant_intent: String,
    pub reliability_score: f64,
    pub evidence_ref: String,
    pub status: String,
}

impl From<SerpPatternState> for NormalizedSerpPatternV1 {
    fn from(pattern: SerpPatternState) -> Self {
        Self {
            serp_pattern_key: pattern.serp_pattern_key,
            query_batch_key: pattern.query_batch_key,
            scope_signature: pattern.scope_signature,
            query: pattern.query,
            pattern_type: pattern.pattern_type,
            dominant_intent: pattern.dominant_intent,
            reliability_score: pattern.reliability_score,
            evidence_ref: pattern.evidence_ref,
            status: pattern.status,
        }
    }
}

impl From<&NormalizedSerpPatternV1> for SerpPatternState {
    fn from(pattern: &NormalizedSerpPatternV1) -> Self {
        Self {
            serp_pattern_key: pattern.serp_pattern_key.clone(),
            query_batch_key: pattern.query_batch_key.clone(),
            scope_signature: pattern.scope_signature.clone(),
            query: pattern.query.clone(),
            pattern_type: pattern.pattern_type.clone(),
            dominant_intent: pattern.dominant_intent.clone(),
            reliability_score: pattern.reliability_score,
            evidence_ref: pattern.evidence_ref.clone(),
            status: pattern.status.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SerpQueryNormalizeOutputV1 {
    pub scope_signature: String,
    pub normalized_query_count: u32,
    pub serp_patterns: Vec<NormalizedSerpPatternV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum SerpNormalizeFailureV1 {
    MissingRunId,
    InvalidScope,
}

impl fmt::Display for SerpNormalizeFailureV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRunId => write!(f, "normalized SERP run_id must be nonempty"),
            Self::InvalidScope => write!(f, "invalid canonical planning scope"),
        }
    }
}

impl Error for SerpNormalizeFailureV1 {}

pub struct NormalizeSerpQueries;

impl Operation for NormalizeSerpQueries {
    type Input = SerpQueryNormalizeInputV1;
    type Output = SerpQueryNormalizeOutputV1;
    type Failure = SerpNormalizeFailureV1;

    fn contract() -> OperationContract {
        pure_contract(
            "seo_steps.normalize_serp_queries",
            "seo_steps.serp_query_normalize_input",
            "seo_steps.serp_query_normalize_output",
        )
    }
}

impl OwnerPureOperation for NormalizeSerpQueries {
    fn evaluate(input: &Self::Input) -> Result<Self::Output, Self::Failure> {
        if input.run_id.trim().is_empty() {
            return Err(SerpNormalizeFailureV1::MissingRunId);
        }
        let result = serp_normalize_step::execute(&SerpNormalizeInputPayload {
            run_id: input.run_id.clone(),
            query_batch_key: input.query_batch_key.clone().unwrap_or_default(),
            scope: input.scope.as_ref().map(SeoScopePayload::from),
            queries: input.queries.clone(),
        })
        .map_err(|_| SerpNormalizeFailureV1::InvalidScope)?;

        Ok(SerpQueryNormalizeOutputV1 {
            scope_signature: result.scope_signature,
            normalized_query_count: result.normalized_query_count,
            serp_patterns: result
                .serp_patterns
                .into_iter()
                .map(NormalizedSerpPatternV1::from)
                .collect(),
        })
    }
}

// PL-03: this is only the deterministic opportunity/coverage decision.
// Durable clusters, Neo4j reads, SERP provenance and persistence are separate
// owner effects, never silently performed by this pure contract.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityTopicSignalV1 {
    pub topic_key: String,
    pub support_refs: Vec<String>,
    pub graph_confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityTripleSignalV1 {
    pub triple_id: String,
    pub object_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityCoverageSignalV1 {
    pub page_node_key: String,
    pub missing_topic_keys: Vec<String>,
    pub graph_confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityPageScopeV1 {
    pub page_node_key: String,
    pub scope_signature: String,
}

/// Only the graph fields that the existing opportunity decision actually
/// reads. Unlike its V1 Temporal transport DTO, there are no ignored fields
/// for stored keyword clusters, graph neighbors or provider routes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityGraphContextV1 {
    pub scope_signature: String,
    pub topic_signals: Vec<OpportunityTopicSignalV1>,
    pub triple_signals: Vec<OpportunityTripleSignalV1>,
    pub coverage_signals: Vec<OpportunityCoverageSignalV1>,
    pub page_scopes: Vec<OpportunityPageScopeV1>,
}

impl From<&OpportunityGraphContextV1> for GraphPlanningContextState {
    fn from(context: &OpportunityGraphContextV1) -> Self {
        Self {
            scope_signature: context.scope_signature.clone(),
            topic_signals: context
                .topic_signals
                .iter()
                .map(|signal| GraphPlanningTopicSignalState {
                    topic_key: signal.topic_key.clone(),
                    support_refs: signal.support_refs.clone(),
                    graph_confidence: signal.graph_confidence,
                    ..Default::default()
                })
                .collect(),
            triple_signals: context
                .triple_signals
                .iter()
                .map(|signal| GraphPlanningTripleSignalState {
                    triple_id: signal.triple_id.clone(),
                    object_key: signal.object_key.clone(),
                    ..Default::default()
                })
                .collect(),
            coverage_signals: context
                .coverage_signals
                .iter()
                .map(|signal| GraphPlanningCoverageSignalState {
                    page_node_key: signal.page_node_key.clone(),
                    missing_topic_keys: signal.missing_topic_keys.clone(),
                    graph_confidence: signal.graph_confidence,
                    ..Default::default()
                })
                .collect(),
            page_nodes: context
                .page_scopes
                .iter()
                .map(|page| PageNodeState {
                    page_node_key: page.page_node_key.clone(),
                    scope_signature: page.scope_signature.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityBuildInputV1 {
    #[serde(default)]
    pub scope: Option<PlanningScopeV1>,
    pub serp_patterns: Vec<NormalizedSerpPatternV1>,
    #[serde(default)]
    pub graph_context: Option<OpportunityGraphContextV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityKeywordClusterV1 {
    pub cluster_key: String,
    pub scope_signature: String,
    pub seed_keyword: String,
    pub dominant_intent: String,
    pub status: String,
    pub cluster_version: u32,
    pub reason_code: String,
    pub topic_keys: Vec<String>,
    pub triple_refs: Vec<String>,
    pub graph_confidence: f64,
    pub support_refs: Vec<String>,
}

impl From<KeywordClusterState> for OpportunityKeywordClusterV1 {
    fn from(cluster: KeywordClusterState) -> Self {
        Self {
            cluster_key: cluster.cluster_key,
            scope_signature: cluster.scope_signature,
            seed_keyword: cluster.seed_keyword,
            dominant_intent: cluster.dominant_intent,
            status: cluster.status,
            cluster_version: cluster.cluster_version,
            reason_code: cluster.reason_code,
            topic_keys: cluster.topic_keys,
            triple_refs: cluster.triple_refs,
            graph_confidence: cluster.graph_confidence,
            support_refs: cluster.support_refs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityContentGapV1 {
    pub content_gap_key: String,
    pub scope_signature: String,
    pub page_node_key: String,
    pub missing_topic: String,
    pub severity: String,
    pub status: String,
    pub reason_code: String,
    pub topic_keys: Vec<String>,
    pub triple_refs: Vec<String>,
    pub graph_confidence: f64,
    pub support_refs: Vec<String>,
}

impl From<ContentGapState> for OpportunityContentGapV1 {
    fn from(gap: ContentGapState) -> Self {
        Self {
            content_gap_key: gap.content_gap_key,
            scope_signature: gap.scope_signature,
            page_node_key: gap.page_node_key,
            missing_topic: gap.missing_topic,
            severity: gap.severity,
            status: gap.status,
            reason_code: gap.reason_code,
            topic_keys: gap.topic_keys,
            triple_refs: gap.triple_refs,
            graph_confidence: gap.graph_confidence,
            support_refs: gap.support_refs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpportunityBuildOutputV1 {
    pub keyword_clusters: Vec<OpportunityKeywordClusterV1>,
    pub content_gaps: Vec<OpportunityContentGapV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpportunityBuildFailureV1 {
    InvalidScope,
}

impl fmt::Display for OpportunityBuildFailureV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid canonical opportunity scope")
    }
}

impl Error for OpportunityBuildFailureV1 {}

pub struct BuildSeoOpportunities;

impl Operation for BuildSeoOpportunities {
    type Input = OpportunityBuildInputV1;
    type Output = OpportunityBuildOutputV1;
    type Failure = OpportunityBuildFailureV1;

    fn contract() -> OperationContract {
        pure_contract(
            "seo_steps.build_seo_opportunities",
            "seo_steps.opportunity_build_input",
            "seo_steps.opportunity_build_output",
        )
    }
}

impl OwnerPureOperation for BuildSeoOpportunities {
    fn evaluate(input: &Self::Input) -> Result<Self::Output, Self::Failure> {
        let output = opportunity_build_step::execute(&OpportunityBuildInputPayload {
            run_id: String::new(), // not read by this pure decision
            scope: input.scope.as_ref().map(SeoScopePayload::from),
            serp_patterns: input
                .serp_patterns
                .iter()
                .map(SerpPatternState::from)
                .collect(),
            graph_context: input
                .graph_context
                .as_ref()
                .map(GraphPlanningContextState::from),
        })
        .map_err(|_| OpportunityBuildFailureV1::InvalidScope)?;
        Ok(OpportunityBuildOutputV1 {
            keyword_clusters: output
                .keyword_clusters
                .into_iter()
                .map(OpportunityKeywordClusterV1::from)
                .collect(),
            content_gaps: output
                .content_gaps
                .into_iter()
                .map(OpportunityContentGapV1::from)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed_schema<T: JsonSchema>() {}

    #[test]
    fn three_operations_have_real_strict_owner_dto_schema_bounds() {
        typed_schema::<ProceduralRulesInputV1>();
        typed_schema::<StrictEntityMentionV1>();
        typed_schema::<ProceduralRulesOutputV1>();
        typed_schema::<ProceduralRoleV1>();
        typed_schema::<SerpQueryNormalizeInputV1>();
        typed_schema::<SerpQueryNormalizeOutputV1>();
        typed_schema::<SerpNormalizeFailureV1>();
        typed_schema::<OpportunityBuildInputV1>();
        typed_schema::<OpportunityGraphContextV1>();
        typed_schema::<OpportunityBuildOutputV1>();
        typed_schema::<OpportunityBuildFailureV1>();
        for contract in [
            ExtractProceduralRules::contract(),
            NormalizeSerpQueries::contract(),
            BuildSeoOpportunities::contract(),
        ] {
            assert_eq!(contract.owner, OperationOwnerId::from("seo_steps"));
            assert_eq!(contract.idempotency, IdempotencyPolicy::Pure);
            assert!(contract.effects.is_empty());
            assert_eq!(contract.inputs.len(), 1);
            assert_eq!(contract.outputs.len(), 1);
            assert!(contract.fingerprint().is_ok());
        }
        assert_eq!(owner_pure_contracts().unwrap().contracts().count(), 3);
    }

    #[test]
    fn independently_extracts_rules_from_typed_mentions() {
        let output = ExtractProceduralRules::evaluate(&ProceduralRulesInputV1 {
            section_id: "section".into(),
            raw_text: "unverified context".into(),
            mentions: vec![StrictEntityMentionV1 {
                raw_text: "80 EUR".into(),
                entity_type: "fee".into(),
                has_numeric: true,
                is_central: true,
                confidence: 0.9,
            }],
        })
        .unwrap();
        assert_eq!(output.rules.len(), 1);
        assert_eq!(output.rules[0].rule_key, "consular_fee");
        assert_eq!(output.rules[0].role_type, ProceduralRoleV1::MustPay);
        assert_eq!(output.rules[0].numeric_tokens, vec!["80 EUR"]);
    }

    #[test]
    fn independent_serp_normalizer_refuses_invalid_scope_and_empty_run() {
        let input = SerpQueryNormalizeInputV1 {
            run_id: "run".into(),
            query_batch_key: None,
            scope: None,
            queries: vec![" price  of visa ".into(), "price of visa".into()],
        };
        let output = NormalizeSerpQueries::evaluate(&input).unwrap();
        // The existing owner algorithm trims and lowercases, but does not
        // collapse internal repeated whitespace. Preserve that exact rule.
        assert_eq!(output.normalized_query_count, 2);
        assert_eq!(output.serp_patterns[0].query, "price  of visa");
        assert_eq!(output.serp_patterns[1].query, "price of visa");
        assert_eq!(output.serp_patterns[0].dominant_intent, "commercial");
        assert_eq!(NormalizeSerpQueries::evaluate(&input).unwrap(), output);

        let missing = SerpQueryNormalizeInputV1 {
            run_id: " ".into(),
            ..input.clone()
        };
        assert_eq!(
            NormalizeSerpQueries::evaluate(&missing),
            Err(SerpNormalizeFailureV1::MissingRunId)
        );

        let invalid = SerpQueryNormalizeInputV1 {
            scope: Some(PlanningScopeV1 {
                market: "alegria-site".into(),
                locale: "ru-RU".into(),
                country_code: "ES".into(),
                visa_type: "tourist".into(),
                applicant_profile: "standard".into(),
                raw_scope_tuple: Some("forged".into()),
                scope_signature: None,
            }),
            ..input
        };
        assert_eq!(
            NormalizeSerpQueries::evaluate(&invalid),
            Err(SerpNormalizeFailureV1::InvalidScope)
        );
    }

    #[test]
    fn opportunity_builder_uses_only_typed_serp_and_graph_signals() {
        let input = OpportunityBuildInputV1 {
            scope: None,
            serp_patterns: vec![NormalizedSerpPatternV1 {
                serp_pattern_key: "pattern".into(),
                query_batch_key: "batch".into(),
                scope_signature: "scope".into(),
                query: "spain visa refusal guide".into(),
                pattern_type: "query_intent".into(),
                dominant_intent: "informational".into(),
                reliability_score: 0.72,
                evidence_ref: "batch".into(),
                status: "active".into(),
            }],
            graph_context: Some(OpportunityGraphContextV1 {
                scope_signature: "scope".into(),
                topic_signals: vec![OpportunityTopicSignalV1 {
                    topic_key: "spain visa refusal guide".into(),
                    support_refs: vec!["section://1".into()],
                    graph_confidence: 0.86,
                }],
                triple_signals: vec![OpportunityTripleSignalV1 {
                    triple_id: "truth-triple".into(),
                    object_key: "topic:spain visa refusal guide".into(),
                }],
                coverage_signals: vec![OpportunityCoverageSignalV1 {
                    page_node_key: "page".into(),
                    missing_topic_keys: vec!["missing-topic".into()],
                    graph_confidence: 0.5,
                }],
                page_scopes: vec![OpportunityPageScopeV1 {
                    page_node_key: "page".into(),
                    scope_signature: "scope".into(),
                }],
            }),
        };
        let output = BuildSeoOpportunities::evaluate(&input).unwrap();
        assert_eq!(output.keyword_clusters.len(), 1);
        assert_eq!(
            output.keyword_clusters[0].reason_code,
            "graph_topic_family_merge"
        );
        assert_eq!(output.keyword_clusters[0].triple_refs, vec!["truth-triple"]);
        assert_eq!(output.content_gaps.len(), 1);
        assert_eq!(output.content_gaps[0].missing_topic, "missing-topic");
        assert_eq!(BuildSeoOpportunities::evaluate(&input).unwrap(), output);

        let invalid = OpportunityBuildInputV1 {
            scope: Some(PlanningScopeV1 {
                market: "alegria-site".into(),
                locale: "ru-RU".into(),
                country_code: "ES".into(),
                visa_type: "tourist".into(),
                applicant_profile: "standard".into(),
                raw_scope_tuple: Some("forged".into()),
                scope_signature: None,
            }),
            ..input
        };
        assert_eq!(
            BuildSeoOpportunities::evaluate(&invalid),
            Err(OpportunityBuildFailureV1::InvalidScope)
        );
    }
}
