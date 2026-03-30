pub use tower::ServiceBuilder;

pub fn default_service_builder() -> ServiceBuilder<tower::layer::util::Identity> {
    ServiceBuilder::new()
}
