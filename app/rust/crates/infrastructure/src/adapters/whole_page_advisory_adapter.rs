use anyhow::Result;
use std::collections::BTreeMap;

use primitives::qdrant_point_id::qdrant_point_id_v1;

use super::qdrant_client_adapter as qdrant;
use super::voyage_api_adapter::VoyageClient;

const COLLECTION_NAME: &str = "whole_page_advisory_prototypes";
const DEFAULT_MODEL: &str = "voyage-4-large";
const DEFAULT_DIMENSION: u32 = 1024;
const DEFAULT_DTYPE: &str = "float";

#[derive(Debug, Clone)]
pub struct WholePageAdvisoryRetrievalHit {
    pub prototype_id: String,
    pub prototype_family: String,
    pub score: f32,
    pub page_mode: String,
    pub dominant_layers: Vec<String>,
    pub country_hints: Vec<String>,
    pub visa_type_hints: Vec<String>,
    pub authority_hints: Vec<String>,
    pub mixed_section_pressure: bool,
}

#[derive(Debug, Clone)]
struct WholePagePrototypeSpec {
    prototype_family: &'static str,
    page_mode: &'static str,
    dominant_layers: &'static [&'static str],
    country_hints: &'static [&'static str],
    visa_type_hints: &'static [&'static str],
    authority_hints: &'static [&'static str],
    mixed_section_pressure: bool,
    prototype_text: &'static str,
}

const PROTOTYPES: &[WholePagePrototypeSpec] = &[
    WholePagePrototypeSpec {
        prototype_family: "content_procedural",
        page_mode: "content_page",
        dominant_layers: &["procedural"],
        country_hints: &[],
        visa_type_hints: &["tourist"],
        authority_hints: &["consulate"],
        mixed_section_pressure: false,
        prototype_text: "Visa document checklist, consular fee, timeline, passport, insurance, application form, consulate requirements, procedural guidance.",
    },
    WholePagePrototypeSpec {
        prototype_family: "content_operational",
        page_mode: "content_page",
        dominant_layers: &["operational"],
        country_hints: &[],
        visa_type_hints: &[],
        authority_hints: &["visa_center"],
        mixed_section_pressure: false,
        prototype_text: "Appointment schedule, holiday closure, office hours, visa center notice, processing delays, operational update, booking rules.",
    },
    WholePagePrototypeSpec {
        prototype_family: "content_editorial",
        page_mode: "content_page",
        dominant_layers: &["editorial"],
        country_hints: &[],
        visa_type_hints: &[],
        authority_hints: &[],
        mixed_section_pressure: false,
        prototype_text: "FAQ, common mistakes, refusal risks, how to prepare, why applications fail, editorial guidance, traveller advice.",
    },
    WholePagePrototypeSpec {
        prototype_family: "content_mixed_procedural_operational",
        page_mode: "content_page",
        dominant_layers: &["procedural", "operational"],
        country_hints: &[],
        visa_type_hints: &["tourist"],
        authority_hints: &["consulate", "visa_center"],
        mixed_section_pressure: true,
        prototype_text: "Tourist visa document checklist plus appointment booking schedule, office hours, submission windows, consulate and VFS procedural and operational guidance.",
    },
    WholePagePrototypeSpec {
        prototype_family: "utility_page",
        page_mode: "utility_page",
        dominant_layers: &[],
        country_hints: &[],
        visa_type_hints: &[],
        authority_hints: &[],
        mixed_section_pressure: false,
        prototype_text: "Privacy policy, cookie settings, login, account access, terms, legal notice, technical utility page.",
    },
    WholePagePrototypeSpec {
        prototype_family: "menu_page",
        page_mode: "menu_page",
        dominant_layers: &[],
        country_hints: &[],
        visa_type_hints: &[],
        authority_hints: &[],
        mixed_section_pressure: false,
        prototype_text: "Navigation menu, breadcrumbs, section links, category links, top navigation, sidebar navigation, menu page.",
    },
    WholePagePrototypeSpec {
        prototype_family: "directory_page",
        page_mode: "directory_page",
        dominant_layers: &["seo"],
        country_hints: &[],
        visa_type_hints: &[],
        authority_hints: &[],
        mixed_section_pressure: false,
        prototype_text: "Directory of visa pages, catalog, sitemap, destination index, page list, navigational directory.",
    },
    WholePagePrototypeSpec {
        prototype_family: "landing_page",
        page_mode: "landing_page",
        dominant_layers: &["commercial"],
        country_hints: &[],
        visa_type_hints: &[],
        authority_hints: &[],
        mixed_section_pressure: false,
        prototype_text: "Consultation, book now, service package, agency offer, turnkey visa support, CTA-heavy landing page.",
    },
    WholePagePrototypeSpec {
        prototype_family: "noisy_footer_nav",
        page_mode: "content_page",
        dominant_layers: &["procedural"],
        country_hints: &[],
        visa_type_hints: &[],
        authority_hints: &[],
        mixed_section_pressure: false,
        prototype_text: "Main visa content with heavy footer links, navigation boilerplate, repeated legal and menu noise, but still primarily procedural content.",
    },
    WholePagePrototypeSpec {
        prototype_family: "country_spain_tourist_authority",
        page_mode: "content_page",
        dominant_layers: &["procedural"],
        country_hints: &["ES"],
        visa_type_hints: &["tourist"],
        authority_hints: &["consulate", "visa_center"],
        mixed_section_pressure: false,
        prototype_text: "Spain tourist visa, consulate of Spain, VFS Spain visa center, Schengen tourist requirements, passport and insurance for Spain.",
    },
    WholePagePrototypeSpec {
        prototype_family: "country_poland_work_authority",
        page_mode: "content_page",
        dominant_layers: &["procedural"],
        country_hints: &["PL"],
        visa_type_hints: &["work"],
        authority_hints: &["consulate", "government"],
        mixed_section_pressure: false,
        prototype_text: "Poland work visa, employment authorization, Polish consulate, ministry guidance, work application documents and filing steps.",
    },
    WholePagePrototypeSpec {
        prototype_family: "country_france_student_authority",
        page_mode: "content_page",
        dominant_layers: &["procedural", "editorial"],
        country_hints: &["FR"],
        visa_type_hints: &["student"],
        authority_hints: &["consulate", "government"],
        mixed_section_pressure: true,
        prototype_text: "France student visa, Campus France preparation, consulate requirements, study application process, editorial preparation guidance.",
    },
];

fn env_model() -> String {
    std::env::var("WHOLE_PAGE_VOYAGE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string())
}

fn parse_csv(payload: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    payload
        .get(key)
        .map(|value| {
            value
                .split('|')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

async fn ensure_seeded(
    client: &qdrant::AlegriaQdrantClient,
    voyage: &VoyageClient,
) -> Result<()> {
    if client.collection_exists(COLLECTION_NAME).await? {
        return Ok(());
    }
    qdrant::ensure_default_dense_collection(client, COLLECTION_NAME, DEFAULT_DIMENSION as u64)
        .await?;

    let texts = PROTOTYPES
        .iter()
        .map(|prototype| prototype.prototype_text)
        .collect::<Vec<_>>();
    let vectors = voyage
        .embed_batch_with_options(
            &texts,
            Some("document"),
            Some(DEFAULT_DIMENSION),
            Some(DEFAULT_DTYPE),
        )
        .await?;

    let points = PROTOTYPES
        .iter()
        .zip(vectors.into_iter())
        .map(|(prototype, vector)| {
            let prototype_id = qdrant_point_id_v1(
                COLLECTION_NAME,
                "whole_page_prototype",
                prototype.prototype_family,
            );
            let mut payload = BTreeMap::new();
            payload.insert("entity_key".to_string(), prototype.prototype_family.to_string());
            payload.insert("prototype_family".to_string(), prototype.prototype_family.to_string());
            payload.insert("page_mode".to_string(), prototype.page_mode.to_string());
            payload.insert(
                "dominant_layers".to_string(),
                prototype.dominant_layers.join("|"),
            );
            payload.insert(
                "country_hints".to_string(),
                prototype.country_hints.join("|"),
            );
            payload.insert(
                "visa_type_hints".to_string(),
                prototype.visa_type_hints.join("|"),
            );
            payload.insert(
                "authority_hints".to_string(),
                prototype.authority_hints.join("|"),
            );
            payload.insert(
                "mixed_section_pressure".to_string(),
                prototype.mixed_section_pressure.to_string(),
            );
            qdrant::DenseEmbeddingPoint {
                point_id: prototype_id,
                vector,
                payload,
            }
        })
        .collect::<Vec<_>>();
    qdrant::upsert_embedding_points(client, COLLECTION_NAME, points).await?;
    Ok(())
}

pub async fn search_whole_page_prototypes(
    page_sketch: &str,
    limit: u64,
) -> Result<Vec<WholePageAdvisoryRetrievalHit>> {
    if page_sketch.trim().is_empty() {
        return Ok(Vec::new());
    }

    let voyage_api_key = match std::env::var("VOYAGE_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(Vec::new()),
    };
    let qdrant_url =
        std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    let voyage = VoyageClient::new(voyage_api_key, env_model());
    let client = qdrant::connect_qdrant(&qdrant_url).await?;
    ensure_seeded(&client, &voyage).await?;

    let vector = voyage
        .embed_batch_with_options(
            &[page_sketch],
            Some("query"),
            Some(DEFAULT_DIMENSION),
            Some(DEFAULT_DTYPE),
        )
        .await?
        .into_iter()
        .next()
        .unwrap_or_default();
    if vector.is_empty() {
        return Ok(Vec::new());
    }

    let points = qdrant::search_dense(&client, COLLECTION_NAME, vector, limit, None).await?;
    Ok(points
        .into_iter()
        .map(|point| {
            let payload = qdrant::scored_point_payload_map(&point);
            WholePageAdvisoryRetrievalHit {
                prototype_id: payload
                    .get("entity_key")
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string()),
                prototype_family: payload
                    .get("prototype_family")
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string()),
                score: point.score,
                page_mode: payload
                    .get("page_mode")
                    .cloned()
                    .unwrap_or_else(|| "content_page".to_string()),
                dominant_layers: parse_csv(&payload, "dominant_layers"),
                country_hints: parse_csv(&payload, "country_hints"),
                visa_type_hints: parse_csv(&payload, "visa_type_hints"),
                authority_hints: parse_csv(&payload, "authority_hints"),
                mixed_section_pressure: payload
                    .get("mixed_section_pressure")
                    .map(|value| value == "true")
                    .unwrap_or(false),
            }
        })
        .collect())
}
