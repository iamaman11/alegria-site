#[async_trait]
impl EditorialGenerationPort for SqlxSeoRuntimeRepository<'_> {
    async fn generate_editorial_draft(
        &self,
        input: &EditorialDraftGenerateInputPayload,
    ) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
        if editorial_llm_adapter::chat_manual_selected(input) {
            return manual_llm_queue::enqueue_editorial(self.pool, input).await;
        }
        editorial_llm_adapter::generate_editorial_draft(input).await
    }
}
