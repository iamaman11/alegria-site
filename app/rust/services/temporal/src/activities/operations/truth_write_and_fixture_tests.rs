// Adjudication is per source candidate; verified.rule_instances is keyed per
// semantic slot. Derive ONE durable outcome per slot, independent of the
// order in which independent sources or rejected proposals are returned.
#[derive(Debug)]
enum SemanticTruthWriteOutcome<'a> {
    Verified(&'a TruthAdjudicationCandidateDecision),
    Disputed,
    Rejected(&'a TruthAdjudicationCandidateDecision),
}

fn semantic_truth_write_outcome<'a>(
    candidates: &[&'a TruthAdjudicationCandidateDecision],
) -> Result<SemanticTruthWriteOutcome<'a>, DomainError> {
    if candidates.is_empty() {
        return Err(DomainError::ValidationFailure {
            message: "cannot write an empty semantic truth identity".to_string(),
        });
    }
    let mut verified = Vec::new();
    let mut unresolved = false;
    for candidate in candidates {
        match candidate.decision.as_str() {
            "verified" if candidate.publish_admissibility == "admissible" => {
                verified.push(*candidate);
            }
            "verified" | "needs_hitl" => unresolved = true,
            "rejected" => (),
            other => {
                return Err(DomainError::ValidationFailure {
                    message: format!("unsupported candidate decision: {other}"),
                });
            }
        }
    }

    // The policy should not produce mixed verified/HITL verdicts, but the
    // write boundary must fail closed even against stale/forged activity data.
    if unresolved {
        return Ok(SemanticTruthWriteOutcome::Disputed);
    }

    if !verified.is_empty() {
        let signatures = verified
            .iter()
            .map(|candidate| candidate.params.stable_signature())
            .collect::<BTreeSet<_>>();
        if signatures.len() != 1 {
            return Ok(SemanticTruthWriteOutcome::Disputed);
        }
        verified.sort_by(|left, right| {
            right
                .confidence
                .total_cmp(&left.confidence)
                .then_with(|| left.rule_candidate_id.cmp(&right.rule_candidate_id))
        });
        return Ok(SemanticTruthWriteOutcome::Verified(verified[0]));
    }

    let rejected = candidates
        .iter()
        .copied()
        .min_by_key(|candidate| candidate.rule_candidate_id.as_str())
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "semantic slot has no admissible candidate".to_string(),
        })?;
    Ok(SemanticTruthWriteOutcome::Rejected(rejected))
}

pub(crate) async fn verified_truth_write_impl(
    acts: &AlegriaActivities,
    input: &VerifiedTruthWriteInput,
) -> Result<VerifiedTruthWriteOutput, DomainError> {
    let mut changed_truth_keys = Vec::new();
    let mut verified_rule_projections = Vec::new();
    let mut demoted_rule_projection_ids = Vec::new();
    let mut verified_rule_count = 0usize;
    let mut demoted_rule_count = 0usize;
    let mut semantic_candidates =
        BTreeMap::<String, Vec<&TruthAdjudicationCandidateDecision>>::new();
    for decision in &input.truth_adjudication.decisions {
        let slot_id = semantic_rule_instance_id_local(
            &input.context_key,
            &decision.role,
            &decision.concept_canonical_key,
        );
        semantic_candidates
            .entry(slot_id)
            .or_default()
            .push(decision);
    }
    let mut semantic_outcomes = BTreeMap::new();
    for (slot_id, candidates) in &semantic_candidates {
        semantic_outcomes.insert(slot_id.clone(), semantic_truth_write_outcome(candidates)?);
    }

    let source_section_ids = input
        .truth_adjudication
        .decisions
        .iter()
        .map(|decision| decision.section_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let raw_sections =
        raw_crawl_adapter::load_raw_sections_by_ids(&acts.pool, &source_section_ids).await?;
    let raw_by_section: BTreeMap<i64, raw_crawl_adapter::RawSectionRecord> = raw_sections
        .into_iter()
        .map(|section| (section.id, section))
        .collect();

    for decision in &input.truth_adjudication.decisions {
        // Reject a forged/stale verification verdict before any SQL mutation.
        // The existing verified-support consumer cannot apply arbitrary scope
        // predicates. Scoped evidence remains available as needs_hitl only.
        if decision.decision == "verified" && decision.evidence_quote.trim().is_empty() {
            return Err(DomainError::ValidationFailure {
                message: format!(
                    "verified candidate {} has no exact source evidence quote",
                    decision.rule_candidate_id,
                ),
            });
        }
        if decision.decision == "verified"
            && !decision
                .scope
                .as_object()
                .is_some_and(|fields| fields.is_empty())
        {
            return Err(DomainError::ValidationFailure {
                message: format!(
                    "scoped candidate {} cannot be globally verified",
                    decision.rule_candidate_id,
                ),
            });
        }
        let raw_section = raw_by_section.get(&decision.section_id).ok_or_else(|| {
            DomainError::ValidationFailure {
                message: format!(
                    "verified truth write missing raw section {} for candidate/source registration",
                    decision.section_id
                ),
            }
        })?;
        raw_crawl_adapter::ensure_source(&acts.pool, raw_section).await?;
        let evidence_quote = if decision.evidence_quote.trim().is_empty() {
            decision.concept_canonical_key.clone()
        } else {
            decision.evidence_quote.clone()
        };
        let span_start = decision.span_start.max(0) as i32;
        let span_end = std::cmp::max(decision.span_end as i32, span_start + 1);
        let severity = decision
            .params
            .as_object()
            .and_then(|params| params.get("severity"))
            .and_then(|value| match value {
                TruthParamValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
            .filter(|value| matches!(*value, "mandatory" | "recommended" | "optional" | "unknown"))
            .unwrap_or("unknown");
        let is_numeric = decision
            .params
            .as_object()
            .map(|params| {
                params.contains_key("amount")
                    || params.contains_key("min_amount")
                    || params.contains_key("max_amount")
                    || params.contains_key("days")
                    || params.contains_key("min_days")
                    || params.contains_key("max_days")
                    || params.contains_key("duration_days")
            })
            .unwrap_or(false);
        let semantic_slot = semantic_rule_instance_id_local(
            &input.context_key,
            &decision.role,
            &decision.concept_canonical_key,
        );
        let slot_outcome = semantic_outcomes.get(&semantic_slot).ok_or_else(|| {
            DomainError::ValidationFailure {
                message: format!("missing write verdict for semantic identity {semantic_slot}"),
            }
        })?;
        let candidate_status = match slot_outcome {
            SemanticTruthWriteOutcome::Disputed if decision.decision == "verified" => "needs_hitl",
            SemanticTruthWriteOutcome::Rejected(_) => "rejected",
            _ => decision.decision.as_str(),
        };
        sqlx_seo_adapter::upsert_extracted_rule_candidate_from_adjudication(
            &acts.pool,
            &sqlx_seo_adapter::VerifiedTruthCandidateWrite {
                rule_candidate_id: decision.rule_candidate_id.clone(),
                context_key: input.context_key.clone(),
                section_id: decision.section_id,
                role: decision.role.clone(),
                concept_canonical_key: decision.concept_canonical_key.clone(),
                evidence_quote: evidence_quote.clone(),
                params: truth_param_value_to_json_local(&decision.params),
                scope: truth_param_value_to_json_local(&decision.scope),
                severity: severity.to_string(),
                is_numeric,
                is_incomplete: decision.completeness_class != "complete",
                confidence: decision.confidence,
                span_start,
                span_end,
                source_key: decision.source_key.clone(),
                source_snapshot_hash: decision.source_snapshot_hash.clone(),
                epistemic_status: candidate_status.to_string(),
            },
        )
        .await?;
    }

    // Exactly one verdict and one outbox event per semantic slot. A rejected
    // source cannot demote a verified fact backed by independent sources.
    for (rule_instance_id, outcome) in semantic_outcomes {
        let changed_key = format!("verified.rule_instance:{rule_instance_id}");
        match outcome {
            SemanticTruthWriteOutcome::Verified(decision) => {
                let role_type = decision.role.to_ascii_lowercase();
                sqlx_seo_adapter::upsert_verified_rule_instance(
                    &acts.pool,
                    &sqlx_seo_adapter::VerifiedRuleInstanceWrite {
                        rule_instance_id: rule_instance_id.clone(),
                        context_key: input.context_key.clone(),
                        rule_type_key: role_type.clone(),
                        concept_key: decision.concept_canonical_key.clone(),
                        role_type: role_type.clone(),
                        params: truth_param_value_to_json_local(&decision.params),
                        scope: truth_param_value_to_json_local(&decision.scope),
                        source_key: decision.source_key.clone(),
                        confidence: decision.confidence,
                        rule_candidate_id: decision.rule_candidate_id.clone(),
                        evidence_section_id: decision.section_id,
                        evidence_quote: decision.evidence_quote.clone(),
                        span_start: decision.span_start.max(0) as i32,
                        span_end: std::cmp::max(
                            decision.span_end as i32,
                            decision.span_start.max(0) as i32 + 1,
                        ),
                        source_snapshot_hash: decision.source_snapshot_hash.clone(),
                        verification_method: decision.verification_method.clone(),
                        adjudication_reason: decision.adjudication_reason.clone(),
                        publish_admissibility: decision.publish_admissibility.clone(),
                        freshness_class: decision.freshness_class.clone(),
                        completeness_class: decision.completeness_class.clone(),
                    },
                )
                .await?;
                verified_rule_count += 1;
                verified_rule_projections.push(VerifiedRuleProjectionCandidate {
                    rule_instance_id: rule_instance_id.clone(),
                    role_type,
                    concept_key: decision.concept_canonical_key.clone(),
                    source_key: decision.source_key.clone(),
                    evidence_quote: decision.evidence_quote.clone(),
                });
            }
            SemanticTruthWriteOutcome::Disputed => {
                sqlx_seo_adapter::demote_verified_rule_instance(
                    &acts.pool,
                    &rule_instance_id,
                    "disputed",
                    "needs_hitl",
                    "truth_write_semantic_guard@1",
                    "mixed_or_conflicting_semantic_adjudication",
                )
                .await?;
                demoted_rule_count += 1;
                demoted_rule_projection_ids.push(rule_instance_id.clone());
            }
            SemanticTruthWriteOutcome::Rejected(decision) => {
                sqlx_seo_adapter::demote_verified_rule_instance(
                    &acts.pool,
                    &rule_instance_id,
                    "deprecated",
                    "not_admissible",
                    &decision.verification_method,
                    &decision.adjudication_reason,
                )
                .await?;
                demoted_rule_count += 1;
                demoted_rule_projection_ids.push(rule_instance_id.clone());
            }
        }
        changed_truth_keys.push(changed_key);
    }

    delete_verified_rules_4_projection(acts, &demoted_rule_projection_ids).await?;
    emit_verified_rules_4_projection(
        acts,
        &input.run_id,
        &input.context_key,
        &verified_rule_projections,
    )
    .await?;

    changed_truth_keys.sort();
    changed_truth_keys.dedup();
    Ok(VerifiedTruthWriteOutput {
        context_key: input.context_key.clone(),
        verified_rule_count,
        demoted_rule_count,
        status: if changed_truth_keys.is_empty() {
            "no_change".to_string()
        } else {
            "written".to_string()
        },
        changed_truth_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    fn candidate_verdict(
        id: &str,
        decision: &str,
        amount: i64,
        confidence: f64,
    ) -> TruthAdjudicationCandidateDecision {
        TruthAdjudicationCandidateDecision {
            section_id: 13,
            page_id: 4,
            rule_candidate_id: id.to_string(),
            role: "FEE_ITEM".to_string(),
            concept_canonical_key: "consular_fee".to_string(),
            params: TruthParamValue::object([("amount", TruthParamValue::Integer(amount))]),
            scope: TruthParamValue::object(Vec::<(String, TruthParamValue)>::new()),
            source_key: format!("https://example.gov/{id}"),
            source_tier: "government".to_string(),
            confidence,
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
            evidence_quote: "consular fee".to_string(),
            span_start: 0,
            span_end: 12,
            source_snapshot_hash: "snapshot".to_string(),
            decision: decision.to_string(),
            publish_admissibility: match decision {
                "verified" => "admissible",
                "needs_hitl" => "needs_hitl",
                _ => "not_admissible",
            }
            .to_string(),
            verification_method: "test".to_string(),
            adjudication_reason: "test_fixture".to_string(),
        }
    }

    #[test]
    fn rejected_source_never_demotes_independently_verified_semantic_slot() {
        let official = candidate_verdict("official", "verified", 80, 0.95);
        let corroborating = candidate_verdict("corroborating", "verified", 80, 0.90);
        let fabricated = candidate_verdict("fabricated", "rejected", 120, 0.99);
        for order in [
            vec![&official, &corroborating, &fabricated],
            vec![&fabricated, &corroborating, &official],
        ] {
            match semantic_truth_write_outcome(&order).unwrap() {
                SemanticTruthWriteOutcome::Verified(candidate) => {
                    assert_eq!(candidate.rule_candidate_id, "official");
                }
                _ => panic!("rejected proposals must not demote verified evidence"),
            }
        }
    }

    #[test]
    fn unresolved_or_conflicting_verified_sources_hold_entire_semantic_slot() {
        let official = candidate_verdict("official", "verified", 80, 0.95);
        let corroborating = candidate_verdict("corroborating", "verified", 80, 0.90);
        let unresolved = candidate_verdict("unresolved", "needs_hitl", 120, 0.91);
        let inconsistent = candidate_verdict("inconsistent", "verified", 120, 0.98);
        assert!(matches!(
            semantic_truth_write_outcome(&[&official, &corroborating, &unresolved]).unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
        assert!(matches!(
            semantic_truth_write_outcome(&[&official, &corroborating, &inconsistent]).unwrap(),
            SemanticTruthWriteOutcome::Disputed
        ));
    }

    #[test]
    fn all_rejected_identity_is_demoted_once_and_unknown_status_fails_closed() {
        let rejected = candidate_verdict("rejected", "rejected", 80, 0.1);
        match semantic_truth_write_outcome(&[&rejected]).unwrap() {
            SemanticTruthWriteOutcome::Rejected(candidate) => {
                assert_eq!(candidate.rule_candidate_id, "rejected");
            }
            _ => panic!("all rejected evidence must demote the slot"),
        }
        let unexpected = candidate_verdict("unknown", "approved", 80, 1.0);
        assert!(semantic_truth_write_outcome(&[&unexpected]).is_err());
    }

    use std::fs;
    use std::path::PathBuf;

    #[derive(Debug, Deserialize)]
    struct WholePageSemanticFixtureSection {
        id: i64,
        heading_path: String,
        section_type: String,
        content_md: String,
        source_url: Option<String>,
        source_domain: Option<String>,
        source_dtype: Option<String>,
        content_hash: Option<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct WholePageSemanticFixtureAdvisoryHit {
        prototype_id: String,
        prototype_family: String,
        score: f32,
        page_mode: String,
        dominant_layers: Vec<String>,
        country_hints: Vec<String>,
        visa_type_hints: Vec<String>,
        authority_hints: Vec<String>,
        mixed_section_pressure: bool,
    }

    #[derive(Debug, Deserialize)]
    struct WholePageSemanticFixtureExpected {
        page_mode_hint: String,
        dominant_layers: Vec<String>,
        country_hints: Vec<String>,
        visa_type_hints: Vec<String>,
        authority_hints: Vec<String>,
        mixed_section_ids: Vec<i64>,
        advisory_model_used: bool,
        advisory_consensus: String,
        required_uncertainty_flags: Vec<String>,
        required_reason_codes: Vec<String>,
        advisory_prototype_families: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    struct WholePageSemanticFixture {
        fixture_id: String,
        description: String,
        sections: Vec<WholePageSemanticFixtureSection>,
        advisory_hits: Option<Vec<WholePageSemanticFixtureAdvisoryHit>>,
        expected: WholePageSemanticFixtureExpected,
    }

    #[derive(Debug, Serialize)]
    struct WholePageSemanticFixtureReport {
        fixture_id: String,
        description: String,
        pass: bool,
        page_mode_hint: String,
        page_mode_confidence: f32,
        dominant_layers: Vec<String>,
        country_hints: Vec<String>,
        visa_type_hints: Vec<String>,
        authority_hints: Vec<String>,
        mixed_section_ids: Vec<i64>,
        advisory_model_used: bool,
        advisory_consensus: String,
        advisory_prototype_families: Vec<String>,
        uncertainty_flags: Vec<String>,
        reason_codes: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    struct WholePageSemanticReportEnvelope {
        artifact_id: String,
        fixture_count: usize,
        fixtures: Vec<WholePageSemanticFixtureReport>,
    }

    fn whole_page_fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/integration_harness/fixtures/whole_page_semantic")
    }

    fn load_whole_page_fixtures() -> Vec<WholePageSemanticFixture> {
        let fixture_dir = whole_page_fixture_dir();
        let mut paths = fs::read_dir(&fixture_dir)
            .expect("read whole-page fixture dir")
            .map(|entry| entry.expect("fixture entry").path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let raw = fs::read_to_string(&path).expect("read whole-page fixture");
                serde_json::from_str::<WholePageSemanticFixture>(&raw)
                    .unwrap_or_else(|err| panic!("parse fixture {}: {err}", path.display()))
            })
            .collect()
    }

    fn build_fixture_sections(
        sections: &[WholePageSemanticFixtureSection],
    ) -> Vec<raw_crawl_adapter::RawSectionRecord> {
        sections
            .iter()
            .map(|section| raw_crawl_adapter::RawSectionRecord {
                id: section.id,
                page_id: 42,
                source_url: section
                    .source_url
                    .clone()
                    .unwrap_or_else(|| "https://example.test/fixture-page".to_string()),
                source_domain: section
                    .source_domain
                    .clone()
                    .unwrap_or_else(|| "example.test".to_string()),
                source_dtype: section
                    .source_dtype
                    .clone()
                    .unwrap_or_else(|| "html".to_string()),
                heading_path: section.heading_path.clone(),
                section_type: section.section_type.clone(),
                content_md: section.content_md.clone(),
                content_hash: section
                    .content_hash
                    .clone()
                    .unwrap_or_else(|| format!("hash-{}", section.id)),
            })
            .collect()
    }

    fn assert_fixture_expectations(
        fixture: &WholePageSemanticFixture,
        actual: &WholePageSemanticPageState,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        if actual.page_mode_hint != fixture.expected.page_mode_hint {
            failures.push(format!(
                "page_mode_hint expected {} got {}",
                fixture.expected.page_mode_hint, actual.page_mode_hint
            ));
        }
        if actual.dominant_layers != fixture.expected.dominant_layers {
            failures.push(format!(
                "dominant_layers expected {:?} got {:?}",
                fixture.expected.dominant_layers, actual.dominant_layers
            ));
        }
        if actual.page_context_profile.country_hints != fixture.expected.country_hints {
            failures.push(format!(
                "country_hints expected {:?} got {:?}",
                fixture.expected.country_hints, actual.page_context_profile.country_hints
            ));
        }
        if actual.page_context_profile.visa_type_hints != fixture.expected.visa_type_hints {
            failures.push(format!(
                "visa_type_hints expected {:?} got {:?}",
                fixture.expected.visa_type_hints, actual.page_context_profile.visa_type_hints
            ));
        }
        if actual.page_context_profile.authority_hints != fixture.expected.authority_hints {
            failures.push(format!(
                "authority_hints expected {:?} got {:?}",
                fixture.expected.authority_hints, actual.page_context_profile.authority_hints
            ));
        }
        if actual.mixed_section_ids != fixture.expected.mixed_section_ids {
            failures.push(format!(
                "mixed_section_ids expected {:?} got {:?}",
                fixture.expected.mixed_section_ids, actual.mixed_section_ids
            ));
        }
        if actual.advisory_model_used != fixture.expected.advisory_model_used {
            failures.push(format!(
                "advisory_model_used expected {} got {}",
                fixture.expected.advisory_model_used, actual.advisory_model_used
            ));
        }
        if actual.advisory_consensus != fixture.expected.advisory_consensus {
            failures.push(format!(
                "advisory_consensus expected {} got {}",
                fixture.expected.advisory_consensus, actual.advisory_consensus
            ));
        }
        if actual.advisory_prototype_families != fixture.expected.advisory_prototype_families {
            failures.push(format!(
                "advisory_prototype_families expected {:?} got {:?}",
                fixture.expected.advisory_prototype_families, actual.advisory_prototype_families
            ));
        }
        for flag in &fixture.expected.required_uncertainty_flags {
            if !actual.uncertainty_flags.contains(flag) {
                failures.push(format!("missing uncertainty flag {flag}"));
            }
        }
        for code in &fixture.expected.required_reason_codes {
            if !actual.reason_codes.contains(code) {
                failures.push(format!("missing reason code {code}"));
            }
        }
        failures
    }

    fn section(id: i64, content_md: &str) -> raw_crawl_adapter::RawSectionRecord {
        raw_crawl_adapter::RawSectionRecord {
            id,
            page_id: 42,
            source_url: "https://example.test/spain-tourist-visa".to_string(),
            source_domain: "example.test".to_string(),
            source_dtype: "html".to_string(),
            heading_path: "Spain tourist visa".to_string(),
            section_type: "content".to_string(),
            content_md: content_md.to_string(),
            content_hash: "hash".to_string(),
        }
    }

    #[test]
    fn whole_page_semantic_fixture_pack_matches_expectations() {
        let fixtures = load_whole_page_fixtures();
        let mut reports = Vec::new();
        let mut failures = Vec::new();

        for fixture in fixtures {
            let sections = build_fixture_sections(&fixture.sections);
            let combined = sections
                .iter()
                .map(|section| section.content_md.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let snapshot = deterministic_whole_page_snapshot(&sections, &combined);
            let advisory_hits = fixture
                .advisory_hits
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(
                    |hit| whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
                        prototype_id: hit.prototype_id,
                        prototype_family: hit.prototype_family,
                        score: hit.score,
                        page_mode: hit.page_mode,
                        dominant_layers: hit.dominant_layers,
                        country_hints: hit.country_hints,
                        visa_type_hints: hit.visa_type_hints,
                        authority_hints: hit.authority_hints,
                        mixed_section_pressure: hit.mixed_section_pressure,
                    },
                )
                .collect::<Vec<_>>();
            let actual = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
            let fixture_failures = assert_fixture_expectations(&fixture, &actual);
            if !fixture_failures.is_empty() {
                failures.push(format!(
                    "{}: {}",
                    fixture.fixture_id,
                    fixture_failures.join("; ")
                ));
            }
            reports.push(WholePageSemanticFixtureReport {
                fixture_id: fixture.fixture_id,
                description: fixture.description,
                pass: fixture_failures.is_empty(),
                page_mode_hint: actual.page_mode_hint,
                page_mode_confidence: (actual.page_mode_confidence * 1000.0).round() / 1000.0,
                dominant_layers: actual.dominant_layers,
                country_hints: actual.page_context_profile.country_hints,
                visa_type_hints: actual.page_context_profile.visa_type_hints,
                authority_hints: actual.page_context_profile.authority_hints,
                mixed_section_ids: actual.mixed_section_ids,
                advisory_model_used: actual.advisory_model_used,
                advisory_consensus: actual.advisory_consensus,
                advisory_prototype_families: actual.advisory_prototype_families,
                uncertainty_flags: actual.uncertainty_flags,
                reason_codes: actual.reason_codes,
            });
        }

        if let Ok(path) = std::env::var("WHOLE_PAGE_SEMANTIC_REPORT_PATH") {
            let report = WholePageSemanticReportEnvelope {
                artifact_id: "whole_page_semantic_fixture_report".to_string(),
                fixture_count: reports.len(),
                fixtures: reports,
            };
            fs::write(
                &path,
                serde_json::to_vec_pretty(&report).expect("serialize whole-page report"),
            )
            .expect("write whole-page report");
        }

        if !failures.is_empty() {
            panic!(
                "whole-page semantic fixture failures:\n{}",
                failures.join("\n")
            );
        }
    }

    #[test]
    fn advisory_conflict_does_not_replace_deterministic_page_mode() {
        let sections = vec![section(
            7,
            "Spain tourist visa requirements. Passport, insurance, fee and appointment details.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "utility".to_string(),
            prototype_family: "utility_page".to_string(),
            score: 0.91,
            page_mode: "utility_page".to_string(),
            dominant_layers: Vec::new(),
            country_hints: Vec::new(),
            visa_type_hints: Vec::new(),
            authority_hints: Vec::new(),
            mixed_section_pressure: false,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert_eq!(fused.page_mode_hint, "content_page");
        assert!(fused
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "advisory_page_mode_conflict:utility_page"));
    }

    #[test]
    fn advisory_only_country_hint_does_not_promote_context_profile() {
        let sections = vec![section(
            9,
            "Student visa guidance with procedural steps and appointment notes.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "poland".to_string(),
            prototype_family: "country_poland_work_authority".to_string(),
            score: 0.88,
            page_mode: "content_page".to_string(),
            dominant_layers: vec!["procedural".to_string()],
            country_hints: vec!["PL".to_string()],
            visa_type_hints: vec!["work".to_string()],
            authority_hints: vec!["government".to_string()],
            mixed_section_pressure: false,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert!(!fused
            .page_context_profile
            .country_hints
            .contains(&"PL".to_string()));
        assert!(fused
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "advisory_only_country_hint:PL"));
    }

    #[test]
    fn low_score_advisory_hit_does_not_change_page_mode_confidence() {
        let sections = vec![section(
            11,
            "Tourist visa document checklist, fee details and passport requirements.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        let baseline_confidence = snapshot.page_mode_confidence;
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "weak".to_string(),
            prototype_family: "content_operational".to_string(),
            score: 0.40,
            page_mode: "content_page".to_string(),
            dominant_layers: vec!["operational".to_string()],
            country_hints: Vec::new(),
            visa_type_hints: Vec::new(),
            authority_hints: Vec::new(),
            mixed_section_pressure: false,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert_eq!(fused.page_mode_hint, "content_page");
        assert!((fused.page_mode_confidence - baseline_confidence).abs() < f32::EPSILON);
        assert!(!fused
            .reason_codes
            .iter()
            .any(|code| code == "high_confidence_advisory_retrieval"));
    }

    #[test]
    fn mixed_pressure_without_section_evidence_does_not_promote_mixed_sections() {
        let sections = vec![section(
            13,
            "Passport and fee guidance only. Consular fee and passport copy.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        assert!(snapshot.mixed_section_ids.is_empty());
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "mixed".to_string(),
            prototype_family: "content_mixed_procedural_operational".to_string(),
            score: 0.89,
            page_mode: "content_page".to_string(),
            dominant_layers: vec!["procedural".to_string(), "operational".to_string()],
            country_hints: Vec::new(),
            visa_type_hints: vec!["tourist".to_string()],
            authority_hints: vec!["consulate".to_string()],
            mixed_section_pressure: true,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert!(fused.mixed_section_ids.is_empty());
        assert!(!fused
            .reason_codes
            .iter()
            .any(|code| code == "advisory_mixed_section_support"));
    }

    #[test]
    fn noisy_footer_heavy_page_stays_content_procedural() {
        let content = "Spain tourist visa requirements. Passport copy, insurance, application form, fee 80 EUR.\n\
Footer links contact privacy menu privacy cookie login terms.\n\
Footer navigation directory menu links help login cookie.";
        let sections = vec![
            section(
                21,
                "Spain tourist visa requirements. Passport copy, insurance, application form, fee 80 EUR.",
            ),
            raw_crawl_adapter::RawSectionRecord {
                id: 22,
                page_id: 42,
                source_url: "https://example.test/spain-tourist-visa".to_string(),
                source_domain: "example.test".to_string(),
                source_dtype: "html".to_string(),
                heading_path: "Footer".to_string(),
                section_type: "footer".to_string(),
                content_md:
                    "Footer links contact privacy menu privacy cookie login terms. Footer navigation directory menu links help login cookie."
                        .to_string(),
                content_hash: "footer".to_string(),
            },
        ];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert_eq!(snapshot.page_mode_hint, "content_page");
        assert!(snapshot.dominant_layers.contains(&"procedural".to_string()));
        assert!(!snapshot.uncertainty_flags.is_empty() || snapshot.page_mode_confidence >= 0.45);
    }

    #[test]
    fn utility_cookie_login_page_is_classified_as_utility() {
        let content =
            "Privacy policy. Cookie settings. Login and account access. Terms and legal notice.";
        let sections = vec![section(31, content)];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert_eq!(snapshot.page_mode_hint, "utility_page");
        assert!(snapshot.page_mode_confidence >= 0.50);
    }

    #[test]
    fn menu_directory_page_is_not_misclassified_as_content() {
        let content = "Breadcrumb menu. Directory of visa pages. Destination index. Category links. Sidebar navigation.";
        let sections = vec![section(41, content)];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert!(
            snapshot.page_mode_hint == "menu_page" || snapshot.page_mode_hint == "directory_page",
            "expected menu/directory classification, got {}",
            snapshot.page_mode_hint
        );
    }

    #[test]
    fn mixed_procedural_editorial_page_sets_multi_layer_flag() {
        let content = "Tourist visa requirements. Passport, insurance and fee 80 EUR. FAQ: why refusals happen and what mistakes to avoid.";
        let sections = vec![section(51, content)];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert!(snapshot.dominant_layers.contains(&"procedural".to_string()));
        assert!(snapshot.dominant_layers.contains(&"editorial".to_string()));
        assert!(snapshot
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "multi_layer_page"));
        assert_eq!(snapshot.mixed_section_ids, vec![51]);
    }
}
