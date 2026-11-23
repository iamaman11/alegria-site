//! Deterministic evidence spans and candidate quality labels.
//! Offsets are UTF-8 byte offsets into the original source document.

use crate::truth_candidates::TruthCandidateValidationResult;

/// Conservative source tier used before registry-backed governance adjudication.
pub fn section_source_tier(source_dtype: &str, source_domain: &str) -> String {
    let dtype = source_dtype.to_ascii_lowercase();
    let domain = source_domain.to_ascii_lowercase();
    if dtype.contains("government")
        || dtype.contains("official")
        || domain.contains(".gov")
        || domain.contains("embassy")
        || domain.contains("consulate")
    {
        "government".to_string()
    } else if dtype.contains("vfs") || domain.contains("vfsglobal") {
        "vfs".to_string()
    } else if dtype.contains("editorial") || domain.contains("news") || domain.contains("blog") {
        "editorial".to_string()
    } else {
        "low_trust".to_string()
    }
}

// Extraction must quote the supporting sentence, not just an isolated
// number/keyword. The returned offsets always refer to the exact UTF-8
// bytes in raw.sections.content_md; no synthetic text is introduced.
fn supporting_sentence_span(
    raw: &str,
    anchor_start: usize,
    anchor_end: usize,
) -> (usize, usize, String) {
    let is_boundary = |offset: usize, ch: char| {
        if matches!(ch, '\n' | '\r' | ';' | '!' | '?') {
            return true;
        }
        ch == '.'
            && raw
                .get(offset + ch.len_utf8()..)
                .is_none_or(|after| after.chars().next().is_none_or(char::is_whitespace))
    };
    let mut left = 0;
    for (idx, ch) in raw[..anchor_start].char_indices() {
        if is_boundary(idx, ch) {
            left = idx + ch.len_utf8();
        }
    }
    let mut right = raw.len();
    for (idx, ch) in raw[anchor_end..].char_indices() {
        let absolute = anchor_end + idx;
        if is_boundary(absolute, ch) {
            right = if matches!(ch, '\n' | '\r' | ';') {
                absolute
            } else {
                absolute + ch.len_utf8()
            };
            break;
        }
    }
    let context = &raw[left..right];
    let leading = context.len() - context.trim_start().len();
    let start = left + leading;
    let quote = context.trim();
    // Never store an entire unpunctuated document as a "quote"; keep the
    // original anchor for manual review if there is no bounded statement.
    if quote.chars().count() > 500 || quote.is_empty() {
        return (
            anchor_start,
            anchor_end,
            raw[anchor_start..anchor_end].to_string(),
        );
    }
    (start, start + quote.len(), quote.to_string())
}

fn literal_token_has_digit_boundaries(raw: &str, start: usize, end: usize) -> bool {
    let first = raw[start..end].chars().next();
    let last = raw[start..end].chars().next_back();
    !(first.is_some_and(|ch| ch.is_ascii_digit())
        && raw[..start]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_ascii_digit())
        || last.is_some_and(|ch| ch.is_ascii_digit())
            && raw[end..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit()))
}

pub fn contains_exact_evidence_token(quote: &str, token: &str) -> bool {
    !token.is_empty()
        && quote
            .match_indices(token)
            .any(|(start, _)| literal_token_has_digit_boundaries(quote, start, start + token.len()))
}

pub fn find_evidence_span(
    raw_text: &str,
    candidates: &[String],
    rule_key: &str,
) -> (usize, usize, String) {
    let mut unsubstantiated = None;
    for candidate in candidates {
        let token = candidate.trim();
        if token.is_empty() {
            continue;
        }
        for (start, _) in raw_text.match_indices(token) {
            let end = start + token.len();
            // An isolated amount or duration cannot be a substring of
            // a different numerical value (80 must not match 180).
            if !literal_token_has_digit_boundaries(raw_text, start, end) {
                continue;
            }
            let found = supporting_sentence_span(raw_text, start, end);
            if deterministic_evidence_has_role_context(rule_key, &found.2) {
                return found;
            }
            if unsubstantiated.is_none() {
                unsubstantiated = Some(found);
            }
        }
    }
    if let Some(found) = unsubstantiated {
        return found;
    }
    // A missing token is not evidence. Preserve a byte-accurate preview for
    // an operator, but candidate_validation must withhold unsupported roles.
    let fallback = raw_text.trim();
    if fallback.is_empty() {
        (0, 0, String::new())
    } else {
        let start = raw_text.len() - raw_text.trim_start().len();
        let end = start + fallback.chars().next().map_or(0, char::len_utf8);
        supporting_sentence_span(raw_text, start, end)
    }
}

pub fn deterministic_evidence_has_role_context(rule_key: &str, quote: &str) -> bool {
    let lower = quote.to_lowercase();
    // A private service charge is not a consular fee even when the number
    // and the word "fee" happen to match. Keep those claims unresolved;
    // source authority is checked independently by canonical adjudication.
    if rule_key == "consular_fee"
        && [
            "service fee",
            "agency service",
            "our service",
            "service charge",
            "agency fee",
            "agent fee",
            "сервисный сбор",
            "сбор за услуги",
            "плата за услуги",
        ]
        .iter()
        .any(|term| lower.contains(term))
    {
        return false;
    }
    let markers: &[&str] = match rule_key {
        "consular_fee" => &["fee", "consular", "сбор", "пошлин", "консульск", "стоим"],
        "processing_time" => &[
            "processing",
            "process time",
            "days to process",
            "срок",
            "рассмотр",
            "обработ",
            "takes",
            "take up to",
        ],
        "passport_required" => &["passport", "паспорт"],
        "insurance_required" => &["insurance", "страхов"],
        _ => return false,
    };
    markers.iter().any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod source_evidence_span_tests {
    use super::*;

    #[test]
    fn numeric_anchor_returns_exact_statement_without_earlier_180() {
        let raw = "Other request: 180 EUR.\nConsular fee is 80 EUR.\nProcessing time is 15 days.";
        let (start, end, quote) = find_evidence_span(raw, &["80".to_string()], "consular_fee");
        assert_eq!(quote, "Consular fee is 80 EUR.");
        assert_eq!(&raw[start..end], quote);
        assert!(deterministic_evidence_has_role_context(
            "consular_fee",
            &quote
        ));
        let (start, end, quote) = find_evidence_span(raw, &["15".to_string()], "processing_time");
        assert_eq!(quote, "Processing time is 15 days.");
        assert_eq!(&raw[start..end], quote);
        assert!(deterministic_evidence_has_role_context(
            "processing_time",
            &quote
        ));
    }

    #[test]
    fn cyrillic_fee_sentence_keeps_utf8_byte_offsets() {
        let raw = "Меню\nКонсульский сбор составляет 35 евро.\nПрочие услуги.";
        let (start, end, quote) = find_evidence_span(raw, &["35".to_string()], "consular_fee");
        assert_eq!(quote, "Консульский сбор составляет 35 евро.");
        assert_eq!(&raw[start..end], quote);
        assert!(start > "Меню".len());
        assert!(deterministic_evidence_has_role_context(
            "consular_fee",
            &quote
        ));
    }

    #[test]
    fn incidental_number_is_not_proof_of_consular_fee() {
        let raw = "Route number 80 arrives today.\nPassport is required.";
        let (start, end, quote) = find_evidence_span(raw, &["80".to_string()], "consular_fee");
        assert_eq!(&raw[start..end], quote);
        assert!(!deterministic_evidence_has_role_context(
            "consular_fee",
            &quote
        ));
        assert!(!deterministic_evidence_has_role_context(
            "processing_time",
            &quote
        ));
    }

    #[test]
    fn duplicated_number_prefers_matching_proposition_not_menu() {
        let raw = "80 people were served.\nConsular fee is 80 EUR.";
        let (start, end, quote) = find_evidence_span(raw, &["80".to_string()], "consular_fee");
        assert_eq!(quote, "Consular fee is 80 EUR.");
        assert_eq!(&raw[start..end], quote);
    }

    #[test]
    fn amount_80_does_not_cite_180_even_in_fee_sentence() {
        let raw = "Consular fee is 180 EUR.";
        let (start, end, quote) = find_evidence_span(raw, &["80".to_string()], "consular_fee");
        assert_eq!(quote, raw);
        assert_eq!(&raw[start..end], quote);
        assert!(!contains_exact_evidence_token(&quote, "80"));
        assert!(contains_exact_evidence_token(&quote, "180"));
    }

    #[test]
    fn missing_token_in_cyrillic_text_is_safe_and_not_synthetic() {
        let raw = "  Паспорт необходимо предъявить.\nПорядок рассмотрения.";
        let (start, end, quote) =
            find_evidence_span(raw, &["insurance".to_string()], "insurance_required");
        assert_eq!(quote, "Паспорт необходимо предъявить.");
        assert_eq!(&raw[start..end], quote);
        assert!(!deterministic_evidence_has_role_context(
            "insurance_required",
            &quote
        ));
    }

    #[test]
    fn agency_service_charge_is_not_consular_fee() {
        for raw in [
            "Our agency service costs 250 EUR.",
            "Our agency service fee is 250 EUR.",
            "Сервисный сбор агентства составляет 250 евро.",
        ] {
            let (start, end, quote) = find_evidence_span(raw, &["250".to_string()], "consular_fee");
            assert_eq!(quote, raw);
            assert_eq!(&raw[start..end], quote);
            assert!(!deterministic_evidence_has_role_context(
                "consular_fee",
                &quote
            ));
        }
        // An agency's genuinely fee-related claim is structurally eligible
        // for source-governance review but never automatically authoritative.
        let agency_summary = "Agency summary says fee is 80 EUR.";
        assert!(deterministic_evidence_has_role_context(
            "consular_fee",
            agency_summary
        ));
    }

    #[test]
    fn unbounded_document_does_not_turn_numeric_token_into_false_proof() {
        let raw = format!("80 {}", "unrelated ".repeat(150));
        let (start, end, quote) = find_evidence_span(&raw, &["80".to_string()], "consular_fee");
        assert_eq!(quote, "80");
        assert_eq!(&raw[start..end], quote);
        assert!(!deterministic_evidence_has_role_context(
            "consular_fee",
            &quote
        ));
    }
}

pub fn classify_candidate_completeness_local(
    validation: &TruthCandidateValidationResult,
) -> String {
    if validation
        .issues
        .iter()
        .any(|issue| incomplete_validator_code(&issue.code))
    {
        "incomplete".to_string()
    } else if validation.epistemic_status == "needs_hitl" {
        "partial".to_string()
    } else {
        "complete".to_string()
    }
}

/// Reloaded candidates store validation issues as prefixed uncertainty flags.
pub fn classify_candidate_completeness_from_flags(
    epistemic_status: &str,
    uncertainty_flags: &[String],
) -> String {
    if uncertainty_flags.iter().any(|flag| {
        flag.strip_prefix("validator:")
            .is_some_and(incomplete_validator_code)
    }) {
        "incomplete".to_string()
    } else if epistemic_status == "needs_hitl" {
        "partial".to_string()
    } else {
        "complete".to_string()
    }
}

fn incomplete_validator_code(code: &str) -> bool {
    matches!(
        code,
        "fee_item_incomplete"
            | "timeline_item_incomplete"
            | "where_to_apply_incomplete"
            | "role_specific_params_missing"
            | "missing_numeric_params"
            | "missing_range_bounds"
            | "candidate_marked_incomplete"
    )
}

pub fn section_uncertainty_flags(raw_text: &str) -> Vec<String> {
    let lowered = raw_text.to_lowercase();
    let mut flags = Vec::new();
    if [
        "archived",
        "archive",
        "outdated",
        "obsolete",
        "retained for record-keeping",
        "retained for record keeping",
        "устар",
        "архив",
        "может быть устар",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        flags.push("stale_source".to_string());
    }
    flags
}

pub fn classify_candidate_freshness_local(uncertainty_flags: &[String]) -> String {
    if uncertainty_flags.iter().any(|flag| {
        flag == "stale_source" || flag == "validator:freshness_or_temporality_ambiguous"
    }) {
        "stale".to_string()
    } else if uncertainty_flags
        .iter()
        .any(|flag| flag == "freshness_ambiguous" || flag == "temporal_ambiguous")
    {
        "watch".to_string()
    } else {
        "fresh".to_string()
    }
}
