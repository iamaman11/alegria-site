async fn render_fixture_source_pages(
    fixtures: &[TruthCertificationFixture],
) -> (
    Vec<stub_servers::StubServerHandle>,
    Vec<(String, serde_json::Value)>,
    Vec<RenderedTruthCertificationPage>,
) {
    let mut source_servers = Vec::new();
    let mut serp_sequence = Vec::new();
    let mut rendered_pages = Vec::new();
    for fixture in fixtures {
        let mut items = Vec::new();
        for (idx, page) in fixture.source_inputs.pages.iter().enumerate() {
            let body = fs::read_to_string(truth_certification_fixtures_dir().join(&page.html_file))
                .unwrap_or_else(|err| {
                    panic!(
                        "failed to read source HTML {} for fixture {}: {err}",
                        page.html_file, fixture.fixture_id
                    )
                });
            let server =
                stub_servers::spawn_text_stub(&page.route, body, "text/html; charset=utf-8")
                    .await
                    .unwrap();
            let url = format!("{}{}", server.base_url, page.route);
            source_servers.push(server);
            rendered_pages.push(RenderedTruthCertificationPage {
                fixture_id: fixture.fixture_id.clone(),
                route: page.route.clone(),
                url: url.clone(),
                page: page.clone(),
            });
            items.push(serde_json::json!({
                "type": "organic",
                "rank_group": idx + 1,
                "rank_absolute": idx + 1,
                "title": page.title,
                "url": url,
                "domain": page.domain,
                "description": page.description
            }));
        }
        serp_sequence.push((
            fixture.fixture_id.clone(),
            serde_json::json!({
                "tasks": [{
                    "result": [{
                        "items": items
                    }]
                }]
            }),
        ));
    }
    (source_servers, serp_sequence, rendered_pages)
}

fn truth_certification_editorial_sequence(
    fixtures: &[TruthCertificationFixture],
) -> Vec<serde_json::Value> {
    fixtures
        .iter()
        .map(|fixture| {
            serde_json::json!({
                "id": format!("chatcmpl-cert-editorial-{}", fixture.fixture_id),
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": fixture.model_stub_inputs.editorial_response.to_string()
                    }
                }]
            })
        })
        .collect()
}

fn truth_certification_truth_sequence(
    fixtures: &[TruthCertificationFixture],
) -> Vec<serde_json::Value> {
    fixtures
        .iter()
        .flat_map(|fixture| {
            let responses = if fixture.model_stub_inputs.extraction_responses.is_empty() {
                vec![serde_json::json!({ "candidates": [] }); fixture.source_inputs.pages.len()]
            } else {
                assert_eq!(
                    fixture.model_stub_inputs.extraction_responses.len(),
                    fixture.source_inputs.pages.len(),
                    "fixture {} must provide one extraction response per source page",
                    fixture.fixture_id
                );
                fixture.model_stub_inputs.extraction_responses.clone()
            };
            responses
                .into_iter()
                .map(|payload| {
                    if payload.get("choices").is_some() {
                        payload
                    } else {
                        local_truth_response(payload)
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn truth_certification_env(
    serp_endpoint: &str,
    truth_endpoint: &str,
    editorial_endpoint: &str,
) -> BTreeMap<String, String> {
    let mut envs = env_overrides::btree_env([
        ("DATAFORSEO_ENDPOINT".to_string(), serp_endpoint.to_string()),
        ("DATAFORSEO_LOGIN".to_string(), "stub-login".to_string()),
        (
            "DATAFORSEO_PASSWORD".to_string(),
            "stub-password".to_string(),
        ),
        ("DATAFORSEO_LANGUAGE_CODE".to_string(), "en".to_string()),
        ("DATAFORSEO_LOCATION_CODE".to_string(), "2840".to_string()),
        ("DATAFORSEO_DEPTH".to_string(), "10".to_string()),
        (
            "SEO_LLM_PROVIDER".to_string(),
            "local_compatible".to_string(),
        ),
        (
            "SEO_LLM_LOCAL_ENDPOINT".to_string(),
            editorial_endpoint.to_string(),
        ),
        (
            "SEO_LLM_LOCAL_MODEL".to_string(),
            "stub-editorial-model".to_string(),
        ),
    ]);
    envs.extend(local_truth_env(truth_endpoint));
    envs.insert(
        "QDRANT_SKIP_COMPATIBILITY_CHECK".to_string(),
        "true".to_string(),
    );
    envs
}
