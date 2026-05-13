use async_trait::async_trait;
use contracts::generated::alegria::read_api::v1::{
    citation_fact, AppointmentRuleParams, CitationFact, ContextBundle, DocumentRequiredParams,
    EligibilityRuleParams, FeeItemParams, FormRequiredParams, RuleRoleTypeV1, SourceCitation,
    StepParams, TimelineItemParams, WhereToApplyParams,
};
use primitives::errors::DomainError;
use runtime_models::RuleParams;
use seo_ports::{
    CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest, CmsReviewPort,
    ContextBundleRepository, DraftRepository, EditorialGenerationPort,
    GlobalNavigationPersistReport, HitlQueuePort, OrganicSerpResponse, OrganicSerpResult,
    PlanningRepository, PublishArtifactRepository, RebuildDependencyEvidence, RebuildRepository,
    SectionTemplateRepository, SemanticLinkCandidate, SemanticLinkSearchPort,
    SeoBuildInputRepository, SerpSearchPort, VerifiedSupportBundleRequest,
    VerifiedSupportRepository,
};

use super::{
    dataforseo_serp_adapter, editorial_llm_adapter, semantic_search_adapter,
    sqlx_adapter::AlegriaPgPool, sqlx_context_bundle_adapter, sqlx_hitl_adapter, sqlx_seo_adapter,
    sqlx_seo_cms_adapter, sqlx_serp_adapter,
};
use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, DraftQaOutputPayload, EditorialDraftGenerateInputPayload,
    EditorialDraftGenerateOutputPayload, FinalizePublishInputPayload, FinalizePublishOutputPayload,
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, HitlDecision,
    HitlTaskContext, IaBuildOutputPayload, LinkRecommendOutputPayload,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RebuildDetectInputPayload, RebuildDetectOutputPayload,
    SectionTemplateBinding, SeoSiteBuildInputPayload, SeoVerifiedFactSupportState,
    SerpIngestInputPayload, SerpIngestOutputPayload, SerpNormalizeInputPayload,
    SerpNormalizeOutputPayload,
};

pub struct SqlxSeoRuntimeRepository<'a> {
    pool: &'a AlegriaPgPool,
}

impl<'a> SqlxSeoRuntimeRepository<'a> {
    pub fn new(pool: &'a AlegriaPgPool) -> Self {
        Self { pool }
    }
}

fn read_role_type(role_type: &str) -> i32 {
    match role_type {
        "must_provide" => RuleRoleTypeV1::MustProvide as i32,
        "must_pay" => RuleRoleTypeV1::MustPay as i32,
        "must_satisfy" => RuleRoleTypeV1::MustSatisfy as i32,
        "allows" => RuleRoleTypeV1::Allows as i32,
        "forbids" => RuleRoleTypeV1::Forbids as i32,
        "timeline" => RuleRoleTypeV1::Timeline as i32,
        "document_required" => RuleRoleTypeV1::DocumentRequired as i32,
        "eligibility_rule" => RuleRoleTypeV1::EligibilityRule as i32,
        "fee_item" => RuleRoleTypeV1::FeeItem as i32,
        "timeline_item" => RuleRoleTypeV1::TimelineItem as i32,
        "where_to_apply" => RuleRoleTypeV1::WhereToApply as i32,
        "appointment_rule" => RuleRoleTypeV1::AppointmentRule as i32,
        "form_required" => RuleRoleTypeV1::FormRequired as i32,
        "step" => RuleRoleTypeV1::Step as i32,
        _ => RuleRoleTypeV1::Unspecified as i32,
    }
}

fn map_params(params: &RuleParams) -> Option<citation_fact::Params> {
    match params {
        RuleParams::None => None,
        RuleParams::Document {
            severity,
            subtype,
            notarization_required,
            translation_required,
            accepts_alternatives,
            conditions_key,
        } => Some(citation_fact::Params::DocParams(DocumentRequiredParams {
            severity: severity.clone(),
            subtype: subtype.clone().unwrap_or_default(),
            notarization_required: *notarization_required,
            translation_required: *translation_required,
            accepts_alternatives: *accepts_alternatives,
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Fee {
            amount,
            currency,
            severity,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::FeeParams(FeeItemParams {
            amount: *amount,
            currency: currency.clone(),
            severity: severity.clone(),
            channel: channel.clone().unwrap_or_default(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Timeline {
            days,
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::TimelineParams(TimelineItemParams {
            days: *days,
            subtype: subtype.clone().unwrap_or_default(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::WhereToApply {
            location_key,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::WhereParams(WhereToApplyParams {
            location_key: location_key.clone(),
            channel: channel.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::EligibilityRule {
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::EligibilityParams(
            EligibilityRuleParams {
                subtype: subtype.clone(),
                severity: severity.clone(),
                conditions_key: conditions_key.clone(),
            },
        )),
        RuleParams::AppointmentRule {
            subtype,
            advance_days,
            conditions_key,
        } => Some(citation_fact::Params::AppointmentParams(
            AppointmentRuleParams {
                subtype: subtype.clone(),
                advance_days: *advance_days,
                conditions_key: conditions_key.clone(),
            },
        )),
        RuleParams::FormRequired {
            form_id,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::FormParams(FormRequiredParams {
            form_id: form_id.clone(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Step {
            step_index,
            subtype,
            conditions_key,
        } => Some(citation_fact::Params::StepParams(StepParams {
            step_index: *step_index,
            subtype: subtype.clone(),
            conditions_key: conditions_key.clone(),
        })),
    }
}

#[async_trait]
impl SeoBuildInputRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_site_build_input(
        &self,
        run_id: &str,
    ) -> Result<SeoSiteBuildInputPayload, DomainError> {
        sqlx_seo_adapter::load_seo_site_build_input(self.pool, run_id).await
    }
}

#[async_trait]
impl VerifiedSupportRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_verified_support_bundle(
        &self,
        request: &VerifiedSupportBundleRequest,
    ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
        sqlx_seo_adapter::load_verified_support_bundle(
            self.pool,
            &request.run_id,
            &request.context_key,
            &request.scope_signature,
            &request.applicant_profile,
        )
        .await
    }
}

#[async_trait]
impl RebuildRepository for SqlxSeoRuntimeRepository<'_> {
    async fn narrow_rebuild_impacts(
        &self,
        changed_truth_keys: &[String],
    ) -> Result<Vec<RebuildDependencyEvidence>, DomainError> {
        let impacted =
            sqlx_seo_adapter::resolve_rebuild_impacts(self.pool, changed_truth_keys).await?;
        Ok(impacted
            .into_iter()
            .map(
                |(page_node_key, reason_package)| RebuildDependencyEvidence {
                    page_node_key,
                    reason_package,
                },
            )
            .collect())
    }

    async fn persist_rebuild_detect_output(
        &self,
        input: &RebuildDetectInputPayload,
        output: &RebuildDetectOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_rebuild_detect_output(self.pool, input, output).await
    }
}

#[async_trait]
impl SerpSearchPort for SqlxSeoRuntimeRepository<'_> {
    async fn fetch_google_organic_live_advanced(
        &self,
        locale: Option<&str>,
        query: &str,
    ) -> Result<Option<OrganicSerpResponse>, DomainError> {
        let Some(config) = dataforseo_serp_adapter::DataForSeoConfig::from_env_with_locale(locale)
        else {
            return Ok(None);
        };
        let client =
            dataforseo_serp_adapter::DataForSeoSerpClient::from_config(config).map_err(|err| {
                DomainError::InfraUnavailable {
                    message: format!("configure DataForSEO client: {err}"),
                }
            })?;
        let response = client
            .google_organic_live_advanced(query)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("DataForSEO organic live advanced failed: {err}"),
            })?;
        Ok(Some(OrganicSerpResponse {
            raw_payload_utf8: response.raw_payload_utf8,
            organic_results: response
                .organic_results
                .into_iter()
                .map(|result| OrganicSerpResult {
                    rank: result.rank,
                    title: result.title,
                    url: result.url,
                    url_norm: result.url_norm,
                    domain_norm: result.domain_norm,
                    snippet: result.snippet,
                })
                .collect(),
        }))
    }
}

#[async_trait]
impl SemanticLinkSearchPort for SqlxSeoRuntimeRepository<'_> {
    async fn search_link_targets(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticLinkCandidate>, DomainError> {
        let results = semantic_search_adapter::search_by_text(
            query,
            "seo_link_targets",
            u64::try_from(limit).map_err(|_| DomainError::ValidationFailure {
                message: format!("semantic link search limit too large: {limit}"),
            })?,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("semantic search failed: {err}"),
        })?;
        Ok(results
            .into_iter()
            .map(|candidate| SemanticLinkCandidate {
                entity_key: candidate.entity_key,
                score: candidate.score,
            })
            .collect())
    }
}

#[async_trait]
impl PlanningRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_serp_ingest_output(
        &self,
        input: &SerpIngestInputPayload,
        output: &SerpIngestOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_serp_ingest_output(self.pool, input, output).await
    }

    async fn persist_live_serp_query_results(
        &self,
        run_id: &str,
        query_batch_key: &str,
        ordinal: usize,
        query: &str,
        response: &OrganicSerpResponse,
    ) -> Result<(), DomainError> {
        let job_id = primitives::hash::content_hash_v1(&format!(
            "{run_id}|{query_batch_key}|{ordinal}|{query}"
        ));
        sqlx_serp_adapter::save_raw_snapshot(
            self.pool,
            &sqlx_serp_adapter::RawSnapshotRecord {
                run_id: run_id.to_string(),
                job_id: job_id.clone(),
                url_norm: query.to_string(),
                query: query.to_string(),
                raw_payload_utf8: response.raw_payload_utf8.clone(),
            },
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("save DataForSEO raw snapshot: {err}"),
        })?;
        let results = response
            .organic_results
            .iter()
            .map(|result| dataforseo_serp_adapter::DataForSeoOrganicResult {
                rank: result.rank,
                title: result.title.clone(),
                url: result.url.clone(),
                url_norm: result.url_norm.clone(),
                domain_norm: result.domain_norm.clone(),
                snippet: result.snippet.clone(),
                raw_json: serde_json::Value::Null,
            })
            .collect::<Vec<_>>();
        sqlx_serp_adapter::save_dataforseo_organic_results_and_enqueue(
            self.pool,
            run_id,
            &job_id,
            query_batch_key,
            &results,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("save DataForSEO organic results: {err}"),
        })?;
        Ok(())
    }

    async fn persist_serp_normalize_output(
        &self,
        input: &SerpNormalizeInputPayload,
        output: &SerpNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_serp_normalize_output(self.pool, input, output).await
    }

    async fn persist_opportunity_build_output(
        &self,
        input: &OpportunityBuildInputPayload,
        output: &OpportunityBuildOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_opportunity_build_output(self.pool, input, output).await
    }

    async fn persist_ia_build_output(
        &self,
        output: &IaBuildOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_ia_build_output(self.pool, output).await
    }

    async fn persist_link_recommend_output(
        &self,
        output: &LinkRecommendOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_link_recommend_output(self.pool, output).await
    }

    async fn persist_global_site_reconcile_output(
        &self,
        input: &GlobalSiteReconcileInputPayload,
        output: &GlobalSiteReconcileOutputPayload,
    ) -> Result<GlobalNavigationPersistReport, DomainError> {
        sqlx_seo_adapter::persist_ia_build_output(
            self.pool,
            &IaBuildOutputPayload {
                page_nodes: output.page_nodes.clone(),
                page_blueprints: Vec::new(),
                cannibalization_conflicts: Vec::new(),
            },
        )
        .await?;
        sqlx_seo_adapter::persist_link_recommend_output(
            self.pool,
            &LinkRecommendOutputPayload {
                link_recommendations: output.link_recommendations.clone(),
            },
        )
        .await?;
        let scope = input.scope.as_ref().cloned().unwrap_or_default();
        let navigation = sqlx_seo_adapter::persist_global_navigation_from_active_pages(
            self.pool,
            &scope.market,
            &scope.locale,
            &input.reconcile_reason,
        )
        .await?;
        Ok(GlobalNavigationPersistReport {
            navigation_tree_key: navigation.navigation_tree_key,
            scope_count: navigation.scope_count,
            page_item_count: navigation.page_item_count,
            silo_group_count: navigation.silo_group_count,
            rebuild_plan_count: navigation.rebuild_plan_count,
        })
    }
}

#[async_trait]
impl SectionTemplateRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_section_templates(
        &self,
        page_type_key: &str,
        dominant_intent: &str,
    ) -> Result<Vec<SectionTemplateBinding>, DomainError> {
        sqlx_seo_adapter::load_section_templates(self.pool, page_type_key, dominant_intent).await
    }
}

#[async_trait]
impl DraftRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_draft_assemble_output(
        &self,
        output: &DraftAssembleOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_assemble_output(self.pool, output).await
    }

    async fn persist_draft_normalize_output(
        &self,
        input: &DraftNormalizeInputPayload,
        output: &DraftNormalizeOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_normalize_output(self.pool, input, output).await
    }

    async fn persist_content_contract_validate_output(
        &self,
        input: &ContentContractValidateInputPayload,
        output: &ContentContractValidateOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_content_contract_validate_output(self.pool, input, output).await
    }

    async fn persist_draft_qa_output(
        &self,
        input: &DraftQaInputPayload,
        output: &DraftQaOutputPayload,
    ) -> Result<(), DomainError> {
        sqlx_seo_adapter::persist_draft_qa_output(self.pool, input, output).await
    }
}

#[async_trait]
impl EditorialGenerationPort for SqlxSeoRuntimeRepository<'_> {
    async fn generate_editorial_draft(
        &self,
        input: &EditorialDraftGenerateInputPayload,
    ) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
        editorial_llm_adapter::generate_editorial_draft(input).await
    }
}

#[async_trait]
impl CmsReviewPort for SqlxSeoRuntimeRepository<'_> {
    async fn persist_cms_publish_output(
        &self,
        input: &CmsPublishInputPayload,
        output: &CmsPublishOutputPayload,
    ) -> Result<CmsPublishOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_cms_publish_output(self.pool, input, output).await
    }

    async fn load_latest_approval_decision(
        &self,
        page_node_key: &str,
        revision_id: &str,
    ) -> Result<Option<CmsApprovalDecision>, DomainError> {
        sqlx_seo_cms_adapter::load_latest_approval_decision(self.pool, page_node_key, revision_id)
            .await
    }
}

#[async_trait]
impl CmsReviewDecisionPort for SqlxSeoRuntimeRepository<'_> {
    async fn apply_human_review_decision(
        &self,
        request: &CmsReviewDecisionRequest,
    ) -> Result<CmsReviewDecisionOutcome, DomainError> {
        sqlx_seo_cms_adapter::apply_human_review_decision(self.pool, request).await
    }
}

#[async_trait]
impl PublishArtifactRepository for SqlxSeoRuntimeRepository<'_> {
    async fn persist_publish_materialize_output(
        &self,
        input: &PublishMaterializeInputPayload,
        output: &PublishMaterializeOutputPayload,
    ) -> Result<PublishMaterializeOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_publish_materialize_output(self.pool, input, output).await
    }

    async fn persist_finalize_publish_output(
        &self,
        input: &FinalizePublishInputPayload,
        output: &FinalizePublishOutputPayload,
    ) -> Result<FinalizePublishOutputPayload, DomainError> {
        sqlx_seo_cms_adapter::persist_finalize_publish_output(self.pool, input, output).await
    }
}

#[async_trait]
impl ContextBundleRepository for SqlxSeoRuntimeRepository<'_> {
    async fn load_context_bundle(&self, context_key: &str) -> Result<ContextBundle, DomainError> {
        let Some(ctx) =
            sqlx_context_bundle_adapter::load_context_bundle_base(self.pool, context_key).await?
        else {
            return Ok(ContextBundle {
                context_key: context_key.to_string(),
                country_code: String::new(),
                visa_family: String::new(),
                visa_subtype: String::new(),
                citizenship_code: String::new(),
                ontology_rules: vec![],
                citation_facts: vec![],
                related_links: vec![],
            });
        };

        let rules_rows =
            sqlx_context_bundle_adapter::load_context_bundle_rules(self.pool, context_key).await?;
        let mut facts = Vec::new();
        for r in rules_rows {
            let citation = if let Some(sk) = r.source_key.clone() {
                Some(SourceCitation {
                    source_key: sk,
                    source_label: r.source_label.unwrap_or_default(),
                    base_url: r.base_url.unwrap_or_default(),
                })
            } else {
                None
            };

            facts.push(CitationFact {
                rule_instance_id: r.rule_instance_id,
                rule_type_key: r.rule_type_key,
                status: r.status,
                effective_from: r.effective_from,
                effective_to: r.effective_to,
                citation,
                role_type: read_role_type(&r.role_type),
                params: map_params(&r.params),
            });
        }

        Ok(ContextBundle {
            context_key: context_key.to_string(),
            country_code: ctx.country_code,
            visa_family: ctx.visa_family,
            visa_subtype: ctx.visa_subtype,
            citizenship_code: ctx.citizenship_code,
            ontology_rules: vec![],
            citation_facts: facts,
            related_links: vec![],
        })
    }
}

#[async_trait]
impl HitlQueuePort for SqlxSeoRuntimeRepository<'_> {
    async fn enqueue_hitl_task(
        &self,
        task_type: &str,
        diagnostics: &HitlTaskContext,
        priority: i32,
    ) -> Result<i64, DomainError> {
        sqlx_hitl_adapter::enqueue_hitl_task(self.pool, task_type, diagnostics, priority).await
    }

    async fn resolve_hitl_task(
        &self,
        task_id: i64,
        resolution: &HitlDecision,
    ) -> Result<(), DomainError> {
        sqlx_hitl_adapter::resolve_hitl_task(self.pool, task_id, resolution).await
    }
}
