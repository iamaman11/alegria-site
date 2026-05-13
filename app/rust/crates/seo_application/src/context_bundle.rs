use contracts::generated::alegria::read_api::v1::ContextBundle;
use primitives::errors::DomainError;
use seo_ports::ContextBundleRepository;

pub async fn load_context_bundle<R: ContextBundleRepository>(
    repo: &R,
    context_key: &str,
) -> Result<ContextBundle, DomainError> {
    repo.load_context_bundle(context_key).await
}
