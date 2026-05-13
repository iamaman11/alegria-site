use contracts::generated::alegria::temporal::v1::{
    DraftAssembleInputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
};

pub fn execute(input: &DraftNormalizeInputPayload) -> DraftNormalizeOutputPayload {
    let assembled = crate::draft_assemble_step::execute(&DraftAssembleInputPayload {
        run_id: input.run_id.clone(),
        page_node: input.page_node.clone(),
        page_blueprint: input.page_blueprint.clone(),
        factual_fragments: input.factual_fragments.clone(),
        verified_support: input.verified_support.clone(),
        required_links: input.required_links.clone(),
        section_templates: input.section_templates.clone(),
        llm_candidate: input.llm_candidate.clone(),
        source_context_chunks: Vec::new(),
    });
    let draft = assembled.draft.unwrap_or_default();
    let mut content_block_plan = input.content_block_plan.clone().unwrap_or_default();
    if content_block_plan.content_block_plan_key.trim().is_empty() {
        content_block_plan = assembled.content_block_plan.unwrap_or_default();
    }
    DraftNormalizeOutputPayload {
        draft: Some(draft),
        content_block_plan: Some(content_block_plan),
    }
}
