use std::sync::Arc;

use contracts::generated::alegria::temporal::v1::{
    HitlPauseInfo, HitlResolutionInput,
};
use infrastructure::adapters::sqlx_adapter::AlegriaPgPool;
use infrastructure::adapters::temporalio_sdk_adapter::{
    activities, ActivityContext, ActivityError,
};

mod content_generation;
mod fact_extraction;
mod operations;
mod runtime;
mod step_catalog;

pub struct AlegriaActivities {
    pub pool: Arc<AlegriaPgPool>,
}

#[allow(dead_code)]
#[activities]
impl AlegriaActivities {
    #[activity]
    pub async fn extract_facts(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "extract_facts", 1, &run_id, || async {
            fact_extraction::extract_facts_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn verify_rules(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "verify_rules", 1, &run_id, || async {
            fact_extraction::verify_rules_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn prepare_hitl_pause(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<HitlPauseInfo, ActivityError> {
        self.execute_step(&run_id, "prepare_hitl_pause", 1, &run_id, || async {
            fact_extraction::prepare_hitl_pause_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn apply_hitl_resolution(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: HitlResolutionInput,
    ) -> Result<String, ActivityError> {
        let run_id = input
            .meta
            .as_ref()
            .map(|m| m.run_id.clone())
            .unwrap_or_default();
        self.execute_step(&run_id, "apply_hitl_resolution", 1, &input, || async {
            fact_extraction::apply_hitl_resolution_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[activity]
    pub async fn persist_and_emit(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "persist_and_emit", 1, &run_id, || async {
            fact_extraction::persist_and_emit_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn generate_content(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "generate_content", 1, &run_id, || async {
            content_generation::generate_content_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn validate_blocks(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "validate_blocks", 1, &run_id, || async {
            content_generation::validate_blocks_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_layer_router_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::layer_router_step::LayerRouterInput,
    ) -> Result<use_cases::layer_router_step::LayerRouterOutput, ActivityError> {
        Ok(step_catalog::run_layer_router(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_entity_span_detection_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::entity_span_detection_step::EntitySpanInput,
    ) -> Result<use_cases::entity_span_detection_step::EntitySpanOutput, ActivityError> {
        Ok(step_catalog::run_entity_span_detection(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_canonical_mapping_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::canonical_mapping_step::CanonicalMappingInput,
    ) -> Result<use_cases::canonical_mapping_step::CanonicalMappingOutput, ActivityError> {
        Ok(step_catalog::run_canonical_mapping(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_procedural_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::procedural_extraction_step::ProceduralExtractionInput,
    ) -> Result<use_cases::procedural_extraction_step::ProceduralExtractionOutput, ActivityError> {
        Ok(step_catalog::run_procedural_extraction(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_operational_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::operational_extraction_step::OperationalExtractionInput,
    ) -> Result<use_cases::operational_extraction_step::OperationalExtractionOutput, ActivityError> {
        Ok(step_catalog::run_operational_extraction(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_editorial_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::editorial_extraction_step::EditorialExtractionInput,
    ) -> Result<use_cases::editorial_extraction_step::EditorialExtractionOutput, ActivityError> {
        Ok(step_catalog::run_editorial_extraction(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_triple_builder_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::triple_builder_step::TripleBuilderInput,
    ) -> Result<use_cases::triple_builder_step::TripleBuilderOutput, ActivityError> {
        Ok(step_catalog::run_triple_builder(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_completeness_judge_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::completeness_judge_step::CompletenessJudgeInput,
    ) -> Result<use_cases::completeness_judge_step::CompletenessJudgeOutput, ActivityError> {
        Ok(step_catalog::run_completeness_judge(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_contradiction_gate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::contradiction_gate_step::ContradictionGateInput,
    ) -> Result<use_cases::contradiction_gate_step::ContradictionGateOutput, ActivityError> {
        Ok(step_catalog::run_contradiction_gate(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_hitl_decision_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::hitl_decision_step::HitlDecisionInput,
    ) -> Result<use_cases::hitl_decision_step::HitlDecisionOutput, ActivityError> {
        Ok(step_catalog::run_hitl_decision(self.as_ref(), &input))
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_neo4j_backwrite_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: use_cases::neo4j_backwrite_step::Neo4jBackwriteInput,
    ) -> Result<use_cases::neo4j_backwrite_step::Neo4jBackwriteOutput, ActivityError> {
        step_catalog::run_neo4j_backwrite(self.as_ref(), &input)
            .await
            .map_err(Self::into_activity_error)
    }

    #[activity]
    pub async fn test_step_prepare(
        self: Arc<Self>,
        _ctx: ActivityContext,
        workflow_id: String,
    ) -> Result<String, ActivityError> {
        Ok(operations::test_step_prepare_impl(&workflow_id))
    }

    #[activity]
    pub async fn test_step_finalize(
        self: Arc<Self>,
        _ctx: ActivityContext,
        prepared_token: String,
    ) -> Result<String, ActivityError> {
        Ok(operations::test_step_finalize_impl(&prepared_token))
    }

    #[activity]
    pub async fn finalize_run(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "finalize_run", 1, &run_id, || async {
            content_generation::finalize_run_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn check_data_freshness(
        self: Arc<Self>,
        _ctx: ActivityContext,
        threshold_input: String,
    ) -> Result<String, ActivityError> {
        let timer = crate::metrics::ActivityTimer::start("check_data_freshness");
        match operations::check_data_freshness_impl(self.as_ref(), &threshold_input).await {
            Ok(report) => {
                timer.record_success();
                Ok(report)
            }
            Err(err) => {
                let class = err.class();
                timer.record_failure(class.as_str());
                Err(Self::activity_error_from_domain(&err))
            }
        }
    }
}
