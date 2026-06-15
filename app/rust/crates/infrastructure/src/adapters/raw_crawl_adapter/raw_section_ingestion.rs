pub async fn load_raw_sections_by_page_ids(
    pool: &PgPool,
    page_ids: &[i64],
) -> std::result::Result<Vec<RawSectionRecord>, primitives::errors::DomainError> {
    if page_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT s.id, s.page_id, p.url AS source_url, p.domain AS source_domain,
                coalesce(p.dtype, '') AS source_dtype,
                coalesce(s.heading_path, '') AS heading_path,
                coalesce(s.section_type, '') AS section_type,
                s.content_md, coalesce(s.content_hash, '') AS content_hash
         FROM raw.sections s
         JOIN raw.pages p ON p.id = s.page_id
         WHERE s.page_id = ANY($1)
         ORDER BY array_position($1, s.page_id), s.section_order, s.id",
    )
    .bind(page_ids)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|row| RawSectionRecord {
            id: row.get("id"),
            page_id: row.get("page_id"),
            source_url: row.get("source_url"),
            source_domain: row.get("source_domain"),
            source_dtype: row.get("source_dtype"),
            heading_path: row.get("heading_path"),
            section_type: row.get("section_type"),
            content_md: row.get("content_md"),
            content_hash: row.get("content_hash"),
        })
        .collect())
}

pub async fn ingest_raw_pages_into_verified(
    pool: &PgPool,
    _run_id: &str,
    context_key: &str,
    raw_page_ids: &[i64],
) -> std::result::Result<RawKnowledgeIngestionReport, primitives::errors::DomainError> {
    if raw_page_ids.is_empty() {
        return Ok(RawKnowledgeIngestionReport::default());
    }
    let mut report = RawKnowledgeIngestionReport {
        raw_page_count: raw_page_ids.len(),
        ..RawKnowledgeIngestionReport::default()
    };
    let sections = load_raw_sections_by_page_ids(pool, raw_page_ids).await?;
    report.raw_section_count = sections.len();
    let expert_core_report =
        super::expert_extraction_core::run_expert_extraction_core(_run_id, context_key, &sections);
    report.expert_blocked_section_count = expert_core_report.blocked_section_count;
    report.expert_needs_hitl_section_count = expert_core_report.needs_hitl_section_count;
    report.expert_verified_ready_section_count = expert_core_report.verified_ready_section_count;
    report.expert_triple_count = expert_core_report.triple_count;
    if let Ok(summary_json) = serde_json::to_string(&expert_core_report) {
        tracing::info!(
            run_id = _run_id,
            context_key,
            raw_section_count = report.raw_section_count,
            expert_extraction_core = %summary_json,
            "evaluated expert extraction core inside raw_knowledge_ingestion"
        );
    }

    for section in &sections {
        if section.content_md.trim().is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO raw.section_context_candidates
             (raw_section_id, context_key, confidence, source_type, mapping_reason, status)
             VALUES ($1, $2, 0.7500, $3, 'raw_knowledge_ingestion@1', 'accepted')
             ON CONFLICT (raw_section_id, context_key) DO UPDATE
             SET confidence = GREATEST(raw.section_context_candidates.confidence, EXCLUDED.confidence),
                 source_type = EXCLUDED.source_type,
                 mapping_reason = EXCLUDED.mapping_reason,
                 status = EXCLUDED.status,
                 updated_at = now()",
        )
        .bind(section.id)
        .bind(context_key)
        .bind(source_type_for_section(section))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        ensure_source(pool, section).await?;

        let Some(extraction) = truth_extraction_llm_adapter::extract_rule_candidates(
            &truth_extraction_llm_adapter::TruthExtractionInput {
                context_key: context_key.to_string(),
                raw_section_id: section.id,
                source_url: section.source_url.clone(),
                source_domain: section.source_domain.clone(),
                heading_path: section.heading_path.clone(),
                raw_text: section.content_md.clone(),
                source_snapshot_hash: section.content_hash.clone(),
            },
        )
        .await?
        else {
            report.extraction_provider_unavailable = true;
            continue;
        };
        if extraction.candidates.is_empty() {
            continue;
        }

        let written =
            persist_extracted_rule_candidates(pool, context_key, section, &extraction).await?;
        report.extracted_rule_count += written;
    }

    let section_ids: Vec<i64> = sections.iter().map(|section| section.id).collect();
    let adjudication =
        adjudicate_persisted_rule_candidates(pool, context_key, &section_ids).await?;
    report.verified_rule_count = adjudication.verified_rule_count;
    report.needs_hitl_candidate_count = adjudication.needs_hitl_candidate_count;
    report
        .changed_truth_keys
        .extend(adjudication.changed_truth_keys);

    report.changed_truth_keys.sort();
    report.changed_truth_keys.dedup();
    Ok(report)
}

