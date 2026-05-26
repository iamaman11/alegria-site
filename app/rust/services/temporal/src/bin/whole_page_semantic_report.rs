use anyhow::{Context, Result};
use clap::Parser;
use infrastructure::adapters::{raw_crawl_adapter, whole_page_advisory_adapter};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
#[path = "../activities/mod.rs"]
mod activities;
#[allow(dead_code)]
#[path = "../metrics.rs"]
mod metrics;

#[derive(Parser, Debug)]
#[command(name = "whole_page_semantic_report")]
#[command(about = "Run whole-page semantic fixture pack and emit machine-readable report")]
struct Cli {
    #[arg(
        long,
        default_value = "app/rust/crates/integration_harness/fixtures/whole_page_semantic"
    )]
    fixture_dir: String,
    #[arg(long, default_value = "/tmp/whole_page_semantic_current.json")]
    report_json: String,
}

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
    failures: Vec<String>,
}

#[derive(Debug, Serialize)]
struct WholePageSemanticReportEnvelope {
    artifact_id: String,
    fixture_count: usize,
    fixtures: Vec<WholePageSemanticFixtureReport>,
}

fn load_fixtures(dir: &PathBuf) -> Result<Vec<WholePageSemanticFixture>> {
    let mut paths = fs::read_dir(dir)
        .with_context(|| format!("read fixture dir {}", dir.display()))?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("list fixture dir {}", dir.display()))?;
    paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("json"));
    paths.sort();
    paths.into_iter()
        .map(|path| {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("read fixture {}", path.display()))?;
            serde_json::from_str::<WholePageSemanticFixture>(&raw)
                .with_context(|| format!("parse fixture {}", path.display()))
        })
        .collect()
}

fn build_sections(
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

fn compare_fixture(
    fixture: &WholePageSemanticFixture,
    actual: &activities::operations::WholePageSemanticPageState,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let fixture_dir = PathBuf::from(cli.fixture_dir);
    let report_path = PathBuf::from(cli.report_json);
    let fixtures = load_fixtures(&fixture_dir)?;
    let mut reports = Vec::new();
    let mut failures = Vec::new();

    for fixture in fixtures {
        let sections = build_sections(&fixture.sections);
        let advisory_hits = fixture
            .advisory_hits
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|hit| whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
                prototype_id: hit.prototype_id,
                prototype_family: hit.prototype_family,
                score: hit.score,
                page_mode: hit.page_mode,
                dominant_layers: hit.dominant_layers,
                country_hints: hit.country_hints,
                visa_type_hints: hit.visa_type_hints,
                authority_hints: hit.authority_hints,
                mixed_section_pressure: hit.mixed_section_pressure,
            })
            .collect::<Vec<_>>();
        let actual = activities::operations::evaluate_whole_page_semantic_fixture(
            &sections,
            &advisory_hits,
        );
        let fixture_failures = compare_fixture(&fixture, &actual);
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
            failures: fixture_failures,
        });
    }

    let report = WholePageSemanticReportEnvelope {
        artifact_id: "whole_page_semantic_fixture_report".to_string(),
        fixture_count: reports.len(),
        fixtures: reports,
    };
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create report dir {}", parent.display()))?;
    }
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("write report {}", report_path.display()))?;
    println!("report: {}", report_path.display());

    if !failures.is_empty() {
        anyhow::bail!("whole-page semantic fixture failures:\n{}", failures.join("\n"));
    }

    println!("WHOLE_PAGE_SEMANTIC_FIXTURES: OK");
    Ok(())
}
