//! True PostgreSQL transaction/HTTP publication E2E without graph or Temporal.
//! All source tests are moved from the broad integration_harness, not copied.

#[cfg(all(test, feature = "e2e"))]
#[allow(unused, warnings)]
mod tests {
    use contracts::generated::alegria::temporal::v1::LinkRecommendationState;
    use infrastructure::adapters::raw_crawl_adapter;
    use integration_data_fixtures::{
        seed_raw_page_with_section, seed_truth_context_and_concept, PostgresHarness,
    };
    use integration_harness::{
        env_overrides, stub_servers,
        truth_test_payloads::{
            local_truth_candidate_payload, local_truth_env, local_truth_response,
        },
    };
    use serial_test::serial;
    use sqlx::Row;
    use uuid::Uuid;

    include!("tests/qa_persistence_tests.rs");
    include!("tests/publication_delivery_tests.rs");
    include!("tests/truth_adjudication_tests.rs");
    include!("tests/cms_review_pg_tests.rs");
    include!("tests/cms_publish_pg_tests.rs");
    include!("tests/cms_cross_scope_pg_tests.rs");
    include!("tests/planning_atomic_pg_tests.rs");
    include!("tests/cms_navigation_pg_tests.rs");
    include!("tests/dataforseo_owner_pg_tests.rs");
    include!("tests/dataforseo_paid_owner_v2_tests.rs");
    include!("tests/external_source_attestation_pg_tests.rs");
    include!("tests/one_step_request_grant_pg_tests.rs");
}
