//! Build editorial blocks from authoritative, admissible PostgreSQL Truth.
//! The reader is replaceable; the role projection and rendering are pure Rust.

use std::collections::{BTreeMap, BTreeSet};

use primitives::errors::DomainError;
use runtime_models::{RuleParams, RuleRoleType};
use seo_domain::{
    canonical_mapping::{self, MappingResult, MatchingStage},
    writer::{self, CitationRule, FaqItem, RuleParamsView, RuleRole},
};
#[cfg(test)]
use seo_domain::canonical_mapping::CanonicalVectorCandidate;
use seo_ports::{
    knowledge::{
        CanonicalVectorLookup, SemanticNeighborCollection,
        SemanticNeighborLookup, SemanticNeighborQuery, SourceContextChunk, SourceContextCollection,
        SourceContextQuery, VerifiedRule, VerifiedRuleProjectionCandidate,
        VerifiedRuleProjectionChange, VerifiedRulesQuery,
    },
    CanonicalVectorSearchPort, SemanticNeighborSearchPort, SourceContextSearchPort,
    VerifiedRuleProjectionPort, VerifiedRulesReader,
};

fn verified_rule_projection_text(
    context_key: &str,
    candidate: &VerifiedRuleProjectionCandidate,
) -> String {
    format!(
        "{} {} {} {} {}",
        context_key,
        candidate.role_type,
        candidate.concept_key,
        candidate.source_key,
        candidate.evidence_quote
    )
    .trim()
    .to_string()
}

/// Truth already decided verification and global-scope eligibility. This
/// operation only coordinates a rebuildable Knowledge search projection.
pub async fn reconcile_verified_rule_projection<P: VerifiedRuleProjectionPort + ?Sized>(
    port: &P,
    change: &VerifiedRuleProjectionChange,
) -> Result<(), DomainError> {
    if !change.delete_rule_ids.is_empty() {
        port.delete_rule_points(&change.run_id, &change.delete_rule_ids)
            .await?;
    }
    if change.upserts.is_empty() {
        return Ok(());
    }
    let texts = change
        .upserts
        .iter()
        .map(|candidate| verified_rule_projection_text(&change.context_key, candidate))
        .collect::<Vec<_>>();
    let Some(embeddings) = port.embed_rule_texts(&change.run_id, &texts).await? else {
        if port.retrieval_required() {
            return Err(DomainError::InfraUnavailable {
                message: format!(
                    "knowledge.project_verified_rules stage=embed run_id={} provider unavailable",
                    change.run_id
                ),
            });
        }
        return Ok(());
    };
    if embeddings.model.trim().is_empty()
        || embeddings.vectors.len() != change.upserts.len()
        || embeddings
            .vectors
            .iter()
            .any(|vector| vector.len() != 1024 || vector.iter().any(|value| !value.is_finite()))
    {
        return Err(DomainError::ContractViolation {
            message: format!(
                "knowledge.project_verified_rules stage=embedding_shape run_id={} expected={} got={}",
                change.run_id, change.upserts.len(), embeddings.vectors.len()
            ),
        });
    }
    port.stage_rule_upserts(change, embeddings).await
}

/// Select editorial evidence neighborhoods in one place. Search hits provide
/// context and diagnostics only: no vector projection can verify a Truth fact.
pub async fn retrieve_source_context<S: SourceContextSearchPort + ?Sized>(
    search: &S,
    request: &SourceContextQuery,
) -> Result<Vec<SourceContextChunk>, DomainError> {
    if request.limit == 0 {
        return Ok(Vec::new());
    }
    if !search.provider_configured() {
        if search.retrieval_required() {
            return Err(DomainError::InfraUnavailable {
                message: format!(
                    "knowledge.source_context stage=provider run_id={} requires a configured provider",
                    request.run_id
                ),
            });
        }
        return Ok(Vec::new());
    }

    let mut chunks = Vec::new();
    let mut seen = BTreeSet::new();
    for collection in [
        SourceContextCollection::VerifiedRules,
        SourceContextCollection::RawContextual,
        SourceContextCollection::EditorialTopics,
        SourceContextCollection::RawStandard,
    ] {
        let found = match search
            .search_source_context_collection(collection, &request.text, request.limit.max(4))
            .await
        {
            Ok(found) => found,
            Err(err) if !search.retrieval_required() => {
                let _ = err;
                continue;
            }
            Err(err) => {
                return Err(DomainError::InfraUnavailable {
                    message: format!(
                        "knowledge.source_context stage=search run_id={} collection={}: {err}",
                        request.run_id,
                        collection.name()
                    ),
                });
            }
        };
        for chunk in found {
            if chunk.collection != collection {
                return Err(DomainError::ContractViolation {
                    message: format!(
                        "knowledge.source_context stage=collection_identity run_id={} requested={} returned={}",
                        request.run_id, collection.name(), chunk.collection.name()
                    ),
                });
            }
            if seen.insert(chunk.chunk_key.clone()) {
                chunks.push(chunk);
            }
            if chunks.len() >= request.limit {
                return Ok(chunks);
            }
        }
    }
    Ok(chunks)
}

/// Read-only Knowledge retrieval for Truth completeness/resolution/conflict
/// diagnostics. Provider errors, empty observations and fallback decisions
/// remain distinguishable; vector hits never change verified Truth status.
pub async fn retrieve_semantic_neighbors<S: SemanticNeighborSearchPort + ?Sized>(
    search: &S,
    request: &SemanticNeighborQuery,
) -> Result<SemanticNeighborLookup, DomainError> {
    if !search.provider_configured() {
        if request.retrieval_required {
            return Err(DomainError::InfraUnavailable {
                message: "retrieval diagnostics require VOYAGE_API_KEY under hard-required retrieval contract"
                    .to_string(),
            });
        }
        return Ok(SemanticNeighborLookup {
            collection_used: None,
            neighbors: Vec::new(),
            reason_codes: vec!["provider_unavailable".into()],
        });
    }
    let primary_name = request.primary.name();
    let fallback_allowed = !(request.contextual_required
        && request.primary == SemanticNeighborCollection::RawContextual);
    let mut reason_codes = Vec::new();
    let primary = search
        .search_semantic_neighbors(&request.text, request.primary, request.limit)
        .await;
    let mut collection_used = Some(primary_name.to_string());
    let mut neighbors = match primary {
        Ok(found) if !found.is_empty() => found,
        Ok(_) => {
            reason_codes.push(format!("{primary_name}:empty"));
            if fallback_allowed {
                if let Some(fallback) = request.fallback {
                    collection_used = Some(fallback.name().to_string());
                    search
                        .search_semantic_neighbors(&request.text, fallback, request.limit)
                        .await
                        .map_err(|err| DomainError::InfraUnavailable {
                            message: format!(
                                "semantic retrieval failed for `{}` after `{primary_name}` empty: {err}",
                                fallback.name()
                            ),
                        })?
                } else {
                    Vec::new()
                }
            } else if request.retrieval_required {
                return Err(DomainError::InfraUnavailable {
                    message: format!(
                        "contextual retrieval is required and `{primary_name}` returned no neighbors"
                    ),
                });
            } else {
                reason_codes.push("contextual_primary_required_no_fallback".into());
                Vec::new()
            }
        }
        Err(err) => {
            if request.retrieval_required {
                return Err(DomainError::InfraUnavailable {
                    message: format!("semantic retrieval failed for `{primary_name}`: {err}"),
                });
            }
            reason_codes.push(format!("{primary_name}:search_failed"));
            if fallback_allowed {
                if let Some(fallback) = request.fallback {
                    collection_used = Some(fallback.name().to_string());
                    search
                        .search_semantic_neighbors(&request.text, fallback, request.limit)
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                reason_codes.push("contextual_primary_required_no_fallback".into());
                Vec::new()
            }
        }
    };
    if neighbors.is_empty() {
        reason_codes.push("semantic_neighbors_empty".into());
    } else {
        neighbors = search
            .rerank_semantic_neighbors(&request.text, neighbors, request.limit)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!(
                    "semantic rerank failed for `{}`: {err}",
                    collection_used.as_deref().unwrap_or_default()
                ),
            })?;
        reason_codes.push("semantic_neighbors_reranked".into());
    }
    Ok(SemanticNeighborLookup {
        collection_used,
        neighbors,
        reason_codes,
    })
}

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
            CanonicalVectorLookup::Candidates(candidates) => {
                canonical_mapping::resolve_vector_candidates(&mapping, &candidates)
            }
        };
        resolved.push(updated);
    }
    Ok(resolved)
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
    use seo_ports::knowledge::{
        SemanticNeighborCandidate, SemanticNeighborCollection, SemanticNeighborQuery,
        SourceContextCollection, SourceContextQuery, VerifiedRuleEmbeddingBatch, VerifiedRulesView,
    };

    struct FakeProjection {
        required: bool,
        embed_available: bool,
        vector_size: usize,
        fail_after_effect_once: std::sync::atomic::AtomicBool,
        deleted: std::sync::Mutex<Vec<String>>,
        applied: std::sync::Mutex<BTreeSet<String>>,
    }

    impl FakeProjection {
        fn new() -> Self {
            Self {
                required: true,
                embed_available: true,
                vector_size: 1024,
                fail_after_effect_once: std::sync::atomic::AtomicBool::new(false),
                deleted: std::sync::Mutex::new(Vec::new()),
                applied: std::sync::Mutex::new(BTreeSet::new()),
            }
        }
    }

    #[async_trait]
    impl VerifiedRuleProjectionPort for FakeProjection {
        fn retrieval_required(&self) -> bool {
            self.required
        }

        async fn delete_rule_points(
            &self,
            _run_id: &str,
            rule_ids: &[String],
        ) -> Result<(), DomainError> {
            self.deleted.lock().unwrap().extend_from_slice(rule_ids);
            for rule in rule_ids {
                self.applied.lock().unwrap().remove(rule);
            }
            Ok(())
        }

        async fn embed_rule_texts(
            &self,
            _run_id: &str,
            _texts: &[String],
        ) -> Result<Option<VerifiedRuleEmbeddingBatch>, DomainError> {
            Ok(self.embed_available.then(|| VerifiedRuleEmbeddingBatch {
                model: "test-model".into(),
                vectors: vec![vec![0.5; self.vector_size]; _texts.len()],
            }))
        }

        async fn stage_rule_upserts(
            &self,
            change: &VerifiedRuleProjectionChange,
            _vectors: VerifiedRuleEmbeddingBatch,
        ) -> Result<(), DomainError> {
            for candidate in &change.upserts {
                self.applied
                    .lock()
                    .unwrap()
                    .insert(candidate.rule_instance_id.clone());
            }
            if self
                .fail_after_effect_once
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(DomainError::InfraUnavailable {
                    message: format!(
                        "simulated lost receipt after effect run_id={}",
                        change.run_id
                    ),
                });
            }
            Ok(())
        }
    }

    fn projection_change() -> VerifiedRuleProjectionChange {
        VerifiedRuleProjectionChange {
            run_id: "run-projection".into(),
            context_key: "ES|tourist||BY".into(),
            delete_rule_ids: Vec::new(),
            upserts: vec![VerifiedRuleProjectionCandidate {
                rule_instance_id: "verified:passport".into(),
                role_type: "document_required".into(),
                concept_key: "passport".into(),
                source_key: "https://example.gov/visa".into(),
                evidence_quote: "A valid passport is required".into(),
            }],
        }
    }

    #[tokio::test]
    async fn verified_projection_port_replay_and_demote_have_one_logical_result() {
        let fake = FakeProjection::new();
        let change = projection_change();
        assert_eq!(
            verified_rule_projection_text(&change.context_key, &change.upserts[0]),
            "ES|tourist||BY document_required passport https://example.gov/visa A valid passport is required"
        );
        fake.fail_after_effect_once
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(reconcile_verified_rule_projection(&fake, &change)
            .await
            .is_err());
        reconcile_verified_rule_projection(&fake, &change)
            .await
            .unwrap();
        assert_eq!(fake.applied.lock().unwrap().len(), 1);
        reconcile_verified_rule_projection(
            &fake,
            &VerifiedRuleProjectionChange {
                delete_rule_ids: vec!["verified:passport".into()],
                upserts: Vec::new(),
                ..change
            },
        )
        .await
        .unwrap();
        assert!(fake.applied.lock().unwrap().is_empty());
        assert_eq!(*fake.deleted.lock().unwrap(), vec!["verified:passport"]);
    }

    #[tokio::test]
    async fn verified_projection_unavailable_provider_is_typed_and_optional_is_noop() {
        let mut fake = FakeProjection::new();
        fake.embed_available = false;
        let change = projection_change();
        let error = reconcile_verified_rule_projection(&fake, &change)
            .await
            .unwrap_err();
        assert_eq!(error.class(), ErrorClass::InfraUnavailable);
        assert!(error.to_string().contains("run-projection"));
        fake.required = false;
        reconcile_verified_rule_projection(&fake, &change)
            .await
            .unwrap();
        assert!(fake.applied.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn verified_projection_rejects_bad_embedding_shape_before_effect() {
        let mut fake = FakeProjection::new();
        fake.vector_size = 4;
        let error = reconcile_verified_rule_projection(&fake, &projection_change())
            .await
            .unwrap_err();
        assert_eq!(error.class(), ErrorClass::ContractViolation);
        assert!(error.to_string().contains("stage=embedding_shape"));
        assert!(fake.applied.lock().unwrap().is_empty());
    }

    struct FakeSourceContextSearch {
        configured: bool,
        required: bool,
        fail: Option<SourceContextCollection>,
        hits: Vec<SourceContextChunk>,
        calls: std::sync::Mutex<Vec<SourceContextCollection>>,
    }

    impl FakeSourceContextSearch {
        fn new(hits: Vec<SourceContextChunk>) -> Self {
            Self {
                configured: true,
                required: false,
                fail: None,
                hits,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SourceContextSearchPort for FakeSourceContextSearch {
        fn provider_configured(&self) -> bool {
            self.configured
        }
        fn retrieval_required(&self) -> bool {
            self.required
        }

        async fn search_source_context_collection(
            &self,
            collection: SourceContextCollection,
            _query: &str,
            limit: usize,
        ) -> Result<Vec<SourceContextChunk>, DomainError> {
            self.calls.lock().unwrap().push(collection);
            if self.fail == Some(collection) {
                return Err(DomainError::InfraUnavailable {
                    message: format!("simulated {} outage", collection.name()),
                });
            }
            Ok(self
                .hits
                .iter()
                .filter(|chunk| chunk.collection == collection)
                .take(limit)
                .cloned()
                .collect())
        }
    }

    fn source_hit(collection: SourceContextCollection, key: &str) -> SourceContextChunk {
        SourceContextChunk {
            collection,
            chunk_key: key.into(),
            source_url: "https://example.gov/source".into(),
            source_domain: "example.gov".into(),
            heading_path: "Visa rules".into(),
            section_type: "visa".into(),
            content_md: "Retrieval is not verified Truth".into(),
            retrieval_score: 0.93,
        }
    }

    fn source_query(limit: usize) -> SourceContextQuery {
        SourceContextQuery {
            run_id: "source-test-run".into(),
            text: "visa evidence".into(),
            limit,
        }
    }

    #[tokio::test]
    async fn source_context_missing_provider_and_zero_limit_obey_required_policy() {
        let mut search = FakeSourceContextSearch::new(Vec::new());
        search.configured = false;
        assert!(retrieve_source_context(&search, &source_query(4))
            .await
            .unwrap()
            .is_empty());
        search.required = true;
        let err = retrieve_source_context(&search, &source_query(4))
            .await
            .unwrap_err();
        assert_eq!(err.class(), ErrorClass::InfraUnavailable);
        assert!(retrieve_source_context(&search, &source_query(0))
            .await
            .unwrap()
            .is_empty());
        assert!(search.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn source_context_prioritizes_collections_deduplicates_and_stops_at_limit() {
        use SourceContextCollection as C;
        let search = FakeSourceContextSearch::new(vec![
            source_hit(C::VerifiedRules, "shared"),
            source_hit(C::RawContextual, "shared"),
            source_hit(C::RawContextual, "context-only"),
            source_hit(C::EditorialTopics, "topic-later"),
        ]);
        let found = retrieve_source_context(&search, &source_query(2))
            .await
            .unwrap();
        assert_eq!(
            found
                .iter()
                .map(|hit| hit.chunk_key.as_str())
                .collect::<Vec<_>>(),
            vec!["shared", "context-only"]
        );
        assert_eq!(found[0].collection.usage_policy(), "verified_fact_support");
        assert_eq!(
            found[1].collection.usage_policy(),
            "contextual_source_neighborhood_not_fact_support"
        );
        assert_eq!(
            *search.calls.lock().unwrap(),
            vec![C::VerifiedRules, C::RawContextual]
        );
    }

    #[tokio::test]
    async fn source_context_optional_failure_continues_but_required_fails_closed() {
        use SourceContextCollection as C;
        let mut search = FakeSourceContextSearch::new(vec![source_hit(C::RawStandard, "raw")]);
        search.fail = Some(C::RawContextual);
        let found = retrieve_source_context(&search, &source_query(4))
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].chunk_key, "raw");
        search.required = true;
        let err = retrieve_source_context(&search, &source_query(4))
            .await
            .unwrap_err();
        assert_eq!(err.class(), ErrorClass::InfraUnavailable);
        assert!(err.to_string().contains("raw_chunks_ctx"));
    }

    #[tokio::test]
    async fn source_context_rejects_mislabeled_projection_without_promoting_truth() {
        let search = FakeSourceContextSearch::new(vec![source_hit(
            SourceContextCollection::VerifiedRules,
            "vector-only",
        )]);
        // The same typed result cannot prove a PostgreSQL Truth rule.
        let found = retrieve_source_context(&search, &source_query(1))
            .await
            .unwrap();
        assert_eq!(found[0].collection, SourceContextCollection::VerifiedRules);
        assert_eq!(found[0].content_md, "Retrieval is not verified Truth");
        // A wrong collection identity from an adapter is a contract failure.
        struct WrongCollection;
        #[async_trait]
        impl SourceContextSearchPort for WrongCollection {
            fn provider_configured(&self) -> bool {
                true
            }
            fn retrieval_required(&self) -> bool {
                true
            }
            async fn search_source_context_collection(
                &self,
                _collection: SourceContextCollection,
                _query: &str,
                _limit: usize,
            ) -> Result<Vec<SourceContextChunk>, DomainError> {
                Ok(vec![source_hit(
                    SourceContextCollection::RawStandard,
                    "wrong",
                )])
            }
        }
        let error = retrieve_source_context(&WrongCollection, &source_query(1))
            .await
            .unwrap_err();
        assert_eq!(error.class(), ErrorClass::ContractViolation);
    }

    #[derive(Clone, Copy)]
    enum SearchReply {
        Empty,
        Hit,
        Failure,
    }

    struct FakeNeighborSearch {
        configured: bool,
        primary: SearchReply,
        fallback: SearchReply,
        rerank_fails: bool,
    }

    #[async_trait]
    impl SemanticNeighborSearchPort for FakeNeighborSearch {
        fn provider_configured(&self) -> bool {
            self.configured
        }

        async fn search_semantic_neighbors(
            &self,
            _query: &str,
            collection: SemanticNeighborCollection,
            _limit: usize,
        ) -> Result<Vec<SemanticNeighborCandidate>, DomainError> {
            let response = if collection == SemanticNeighborCollection::RawStandard {
                self.fallback
            } else {
                self.primary
            };
            match response {
                SearchReply::Empty => Ok(Vec::new()),
                SearchReply::Hit => Ok(vec![SemanticNeighborCandidate {
                    entity_key: "source:42".into(),
                    score: 0.91,
                    retrieval_text: "diagnostic only".into(),
                }]),
                SearchReply::Failure => Err(DomainError::InfraUnavailable {
                    message: "simulated Qdrant outage".into(),
                }),
            }
        }

        async fn rerank_semantic_neighbors(
            &self,
            _query: &str,
            mut candidates: Vec<SemanticNeighborCandidate>,
            _limit: usize,
        ) -> Result<Vec<SemanticNeighborCandidate>, DomainError> {
            if self.rerank_fails {
                return Err(DomainError::InfraUnavailable {
                    message: "simulated Voyage rerank outage".into(),
                });
            }
            for candidate in &mut candidates {
                candidate.score = 0.96;
            }
            Ok(candidates)
        }
    }

    fn neighbor_query(required: bool, contextual: bool) -> SemanticNeighborQuery {
        SemanticNeighborQuery {
            text: "Official evidence".into(),
            primary: SemanticNeighborCollection::RawContextual,
            fallback: Some(SemanticNeighborCollection::RawStandard),
            limit: 4,
            retrieval_required: required,
            contextual_required: contextual,
        }
    }

    fn neighbor_fake(primary: SearchReply, fallback: SearchReply) -> FakeNeighborSearch {
        FakeNeighborSearch {
            configured: true,
            primary,
            fallback,
            rerank_fails: false,
        }
    }

    #[tokio::test]
    async fn neighbor_provider_absence_preserves_typed_required_gate() {
        let fake = FakeNeighborSearch {
            configured: false,
            ..neighbor_fake(SearchReply::Hit, SearchReply::Hit)
        };
        let optional = retrieve_semantic_neighbors(&fake, &neighbor_query(false, false))
            .await
            .unwrap();
        assert!(optional.collection_used.is_none());
        assert!(optional.neighbors.is_empty());
        assert_eq!(optional.reason_codes, vec!["provider_unavailable"]);
        let required = retrieve_semantic_neighbors(&fake, &neighbor_query(true, false))
            .await
            .unwrap_err();
        assert_eq!(required.class(), ErrorClass::InfraUnavailable);
    }

    #[tokio::test]
    async fn neighbor_empty_primary_uses_typed_fallback_and_rerank() {
        let lookup = retrieve_semantic_neighbors(
            &neighbor_fake(SearchReply::Empty, SearchReply::Hit),
            &neighbor_query(true, false),
        )
        .await
        .unwrap();
        assert_eq!(lookup.collection_used.as_deref(), Some("raw_chunks_4"));
        assert_eq!(lookup.neighbors[0].entity_key, "source:42");
        assert_eq!(lookup.neighbors[0].score, 0.96);
        assert_eq!(
            lookup.reason_codes,
            vec!["raw_chunks_ctx:empty", "semantic_neighbors_reranked"]
        );
    }

    #[tokio::test]
    async fn contextual_required_never_falls_back_on_empty_or_failure() {
        let empty = neighbor_fake(SearchReply::Empty, SearchReply::Hit);
        let err = retrieve_semantic_neighbors(&empty, &neighbor_query(true, true))
            .await
            .unwrap_err();
        assert_eq!(err.class(), ErrorClass::InfraUnavailable);
        assert!(err.to_string().contains("returned no neighbors"));

        let failed = neighbor_fake(SearchReply::Failure, SearchReply::Hit);
        let optional = retrieve_semantic_neighbors(&failed, &neighbor_query(false, true))
            .await
            .unwrap();
        assert!(optional.neighbors.is_empty());
        assert_eq!(optional.collection_used.as_deref(), Some("raw_chunks_ctx"));
        assert!(optional
            .reason_codes
            .contains(&"contextual_primary_required_no_fallback".into()));
    }

    #[tokio::test]
    async fn search_failure_can_be_best_effort_but_rerank_failure_cannot() {
        let failed = neighbor_fake(SearchReply::Failure, SearchReply::Hit);
        let lookup = retrieve_semantic_neighbors(&failed, &neighbor_query(false, false))
            .await
            .unwrap();
        assert_eq!(lookup.collection_used.as_deref(), Some("raw_chunks_4"));
        assert!(lookup
            .reason_codes
            .contains(&"raw_chunks_ctx:search_failed".into()));
        assert_eq!(lookup.neighbors.len(), 1);

        let mut rerank_failed = neighbor_fake(SearchReply::Hit, SearchReply::Empty);
        rerank_failed.rerank_fails = true;
        let err = retrieve_semantic_neighbors(&rerank_failed, &neighbor_query(false, false))
            .await
            .unwrap_err();
        assert_eq!(err.class(), ErrorClass::InfraUnavailable);
        assert!(err.to_string().contains("semantic rerank failed"));
    }

    #[tokio::test]
    async fn empty_primary_requires_successful_fallback_but_both_failed_is_diagnostic() {
        let failed_fallback = neighbor_fake(SearchReply::Empty, SearchReply::Failure);
        let err = retrieve_semantic_neighbors(&failed_fallback, &neighbor_query(false, false))
            .await
            .unwrap_err();
        assert_eq!(err.class(), ErrorClass::InfraUnavailable);
        assert!(err.to_string().contains("after `raw_chunks_ctx` empty"));

        let both_failed = neighbor_fake(SearchReply::Failure, SearchReply::Failure);
        let outcome = retrieve_semantic_neighbors(&both_failed, &neighbor_query(false, false))
            .await
            .unwrap();
        assert!(outcome.neighbors.is_empty());
        assert_eq!(outcome.collection_used.as_deref(), Some("raw_chunks_4"));
        assert_eq!(
            outcome.reason_codes,
            vec!["raw_chunks_ctx:search_failed", "semantic_neighbors_empty"]
        );
    }

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
