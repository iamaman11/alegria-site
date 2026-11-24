//! Pure rules at the boundary where extracted proposals become Truth candidates.
//!
//! Providers and durable inboxes own JSON/HTTP/SQL; Temporal owns ordering and
//! retry. Neither is allowed to invent evidence, fee or epistemic policy.

use crate::truth_candidates::TruthParamValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDisposition {
    Structured,
    NeedsHitl,
    Rejected,
}

impl CandidateDisposition {
    pub fn from_wire(value: &str) -> Result<Self, &'static str> {
        match value {
            "structured" => Ok(Self::Structured),
            "needs_hitl" => Ok(Self::NeedsHitl),
            "rejected" => Ok(Self::Rejected),
            _ => Err("unknown Truth candidate epistemic status"),
        }
    }

    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::NeedsHitl => "needs_hitl",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CandidateGates {
    pub role_context_supported: bool,
    pub ontology_needs_hitl: bool,
    pub schema_invalid: bool,
}

/// A schema rejection dominates review requirements. Existing rejections stay
/// rejected; unsupported contextual evidence and ontology uncertainty demand
/// review without downgrading an already rejected candidate.
pub fn decide_disposition(
    initial: CandidateDisposition,
    gates: CandidateGates,
) -> CandidateDisposition {
    if initial == CandidateDisposition::Rejected || gates.schema_invalid {
        CandidateDisposition::Rejected
    } else if !gates.role_context_supported || gates.ontology_needs_hitl {
        CandidateDisposition::NeedsHitl
    } else {
        initial
    }
}

pub struct EvidenceSection<'a> {
    pub id: i64,
    pub text: &'a str,
}

pub struct SuggestedEvidence<'a> {
    pub role: &'a str,
    pub concept_key: &'a str,
    pub evidence_quote: &'a str,
    pub evidence_section_id: Option<i64>,
    pub span_start: usize,
    pub span_end: usize,
    pub raw_mention: &'a str,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedEvidence {
    pub role: &'static str,
    pub raw_mention: String,
}

/// The original source bytes, not a provider's authority claim, determine
/// whether a suggestion may proceed to deeper Truth validation.
pub fn validate_evidence(
    source: &EvidenceSection<'_>,
    claim: &SuggestedEvidence<'_>,
) -> Result<AcceptedEvidence, &'static str> {
    let role = match claim.role.trim().to_ascii_uppercase().as_str() {
        "DOCUMENT_REQUIRED" => "DOCUMENT_REQUIRED",
        "ELIGIBILITY_RULE" => "ELIGIBILITY_RULE",
        "FEE_ITEM" => "FEE_ITEM",
        "TIMELINE_ITEM" => "TIMELINE_ITEM",
        "WHERE_TO_APPLY" => "WHERE_TO_APPLY",
        "APPOINTMENT_RULE" => "APPOINTMENT_RULE",
        "FORM_REQUIRED" => "FORM_REQUIRED",
        "STEP" => "STEP",
        _ => return Err("unsupported candidate role"),
    };
    if claim.concept_key.trim().is_empty() {
        return Err("concept_canonical_key is required");
    }
    if claim.evidence_quote.trim().is_empty() {
        return Err("evidence_quote is required");
    }
    if claim.evidence_section_id != Some(source.id) {
        return Err("evidence_section_id does not match source section");
    }
    if claim.span_start >= claim.span_end {
        return Err("evidence span must be nonempty");
    }
    if claim.span_end > source.text.len()
        || !source.text.is_char_boundary(claim.span_start)
        || !source.text.is_char_boundary(claim.span_end)
    {
        return Err("evidence span is outside source text or not UTF-8 aligned");
    }
    let snippet = source
        .text
        .get(claim.span_start..claim.span_end)
        .ok_or("evidence span is invalid")?;
    if snippet.trim().is_empty() || claim.evidence_quote != snippet {
        return Err("evidence_quote must match the exact source bytes");
    }
    if !claim.confidence.is_finite() || claim.confidence <= 0.0 || claim.confidence > 1.0 {
        return Err("confidence must be in (0, 1]");
    }
    Ok(AcceptedEvidence {
        role,
        raw_mention: if claim.raw_mention.trim().is_empty() {
            snippet.to_string()
        } else {
            claim.raw_mention.to_string()
        },
    })
}

pub fn normalized_proposal_metadata(severity: &str, derivation_type: &str) -> (String, String) {
    (
        if severity.trim().is_empty() {
            "unknown"
        } else {
            severity
        }
        .to_string(),
        if derivation_type.trim().is_empty() {
            "direct"
        } else {
            derivation_type
        }
        .to_string(),
    )
}

/// Canonicalize only monetary fee fields. Other integer claims, notably
/// processing days, keep their original type and stable identity.
pub fn canonicalize_fee_parameters(
    role: &str,
    params: &mut TruthParamValue,
) -> Result<(), &'static str> {
    if role != "FEE_ITEM" {
        return Ok(());
    }
    if let TruthParamValue::Object(fields) = params {
        for name in ["amount", "min_amount", "max_amount"] {
            if let Some(value) = fields.get_mut(name) {
                if let TruthParamValue::Integer(integer) = value {
                    let original = *integer;
                    // Every integer through 2^53 is exactly representable in f64.
                    if !(-9_007_199_254_740_992..=9_007_199_254_740_992).contains(&original) {
                        return Err(name);
                    }
                    *value = TruthParamValue::Decimal(original as f64);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> EvidenceSection<'static> {
        EvidenceSection {
            id: 7,
            text: "Паспорт обязателен. Сбор 80 EUR.",
        }
    }

    fn proposed() -> SuggestedEvidence<'static> {
        SuggestedEvidence {
            role: " fee_item ",
            concept_key: "consular_fee",
            evidence_quote: "Сбор 80 EUR",
            evidence_section_id: Some(7),
            span_start: 37,
            span_end: 50,
            raw_mention: "",
            confidence: 0.9,
        }
    }

    #[test]
    fn provider_evidence_must_be_source_bound_and_utf8_aligned() {
        let source = source();
        let start = source.text.find("Сбор 80 EUR").unwrap();
        let end = start + "Сбор 80 EUR".len();
        let mut claim = proposed();
        claim.span_start = start;
        claim.span_end = end;
        let accepted = validate_evidence(&source, &claim).unwrap();
        assert_eq!(accepted.role, "FEE_ITEM");
        assert_eq!(accepted.raw_mention, "Сбор 80 EUR");

        claim.evidence_section_id = Some(8);
        assert_eq!(
            validate_evidence(&source, &claim).unwrap_err(),
            "evidence_section_id does not match source section"
        );
        claim.evidence_section_id = Some(7);
        claim.span_start = 1;
        assert_eq!(
            validate_evidence(&source, &claim).unwrap_err(),
            "evidence span is outside source text or not UTF-8 aligned"
        );
        claim.span_start = start;
        claim.evidence_quote = "Сбор 180 EUR";
        assert_eq!(
            validate_evidence(&source, &claim).unwrap_err(),
            "evidence_quote must match the exact source bytes"
        );
        claim.evidence_quote = "Сбор 80 EUR";
        claim.role = "invented_role";
        assert_eq!(
            validate_evidence(&source, &claim).unwrap_err(),
            "unsupported candidate role"
        );
        claim.role = "FEE_ITEM";
        claim.confidence = f64::NAN;
        assert_eq!(
            validate_evidence(&source, &claim).unwrap_err(),
            "confidence must be in (0, 1]"
        );
        claim.confidence = 0.9;
        claim.concept_key = "";
        assert_eq!(
            validate_evidence(&source, &claim).unwrap_err(),
            "concept_canonical_key is required"
        );
    }

    #[test]
    fn status_precedence_is_typed_and_never_promotes_a_rejection() {
        let all = [
            CandidateDisposition::Structured,
            CandidateDisposition::NeedsHitl,
            CandidateDisposition::Rejected,
        ];
        for status in all {
            assert_eq!(
                CandidateDisposition::from_wire(status.as_wire()).unwrap(),
                status
            );
            assert_eq!(
                decide_disposition(
                    status,
                    CandidateGates {
                        role_context_supported: true,
                        ontology_needs_hitl: false,
                        schema_invalid: true,
                    }
                ),
                CandidateDisposition::Rejected
            );
            for role_context_supported in [false, true] {
                for ontology_needs_hitl in [false, true] {
                    let chosen = decide_disposition(
                        status,
                        CandidateGates {
                            role_context_supported,
                            ontology_needs_hitl,
                            schema_invalid: false,
                        },
                    );
                    let expected = if status == CandidateDisposition::Rejected {
                        CandidateDisposition::Rejected
                    } else if !role_context_supported || ontology_needs_hitl {
                        CandidateDisposition::NeedsHitl
                    } else {
                        status
                    };
                    assert_eq!(chosen, expected);
                }
            }
        }
        assert_eq!(
            decide_disposition(
                CandidateDisposition::Structured,
                CandidateGates {
                    role_context_supported: false,
                    ontology_needs_hitl: false,
                    schema_invalid: false,
                }
            ),
            CandidateDisposition::NeedsHitl
        );
        assert!(CandidateDisposition::from_wire("verified").is_err());
    }

    #[test]
    fn monetary_fee_normalization_preserves_exactness_and_unrelated_claims() {
        let mut fee = TruthParamValue::object([
            ("amount", TruthParamValue::Integer(80)),
            ("min_amount", TruthParamValue::Integer(35)),
            ("max_amount", TruthParamValue::Integer(9_007_199_254_740_992)),
        ]);
        canonicalize_fee_parameters("FEE_ITEM", &mut fee).unwrap();
        let fields = fee.as_object().unwrap();
        assert_eq!(fields.get("amount"), Some(&TruthParamValue::Decimal(80.0)));
        assert_eq!(fields.get("min_amount"), Some(&TruthParamValue::Decimal(35.0)));
        assert_eq!(
            fields.get("max_amount"),
            Some(&TruthParamValue::Decimal(9_007_199_254_740_992.0))
        );

        let mut day = TruthParamValue::object([("days", TruthParamValue::Integer(15))]);
        canonicalize_fee_parameters("TIMELINE_ITEM", &mut day).unwrap();
        assert_eq!(
            day.as_object().unwrap().get("days"),
            Some(&TruthParamValue::Integer(15))
        );

        let mut fee_with_days = TruthParamValue::object([
            ("amount", TruthParamValue::Integer(80)),
            ("days", TruthParamValue::Integer(15)),
        ]);
        canonicalize_fee_parameters("FEE_ITEM", &mut fee_with_days).unwrap();
        let fields = fee_with_days.as_object().unwrap();
        assert_eq!(fields.get("amount"), Some(&TruthParamValue::Decimal(80.0)));
        assert_eq!(fields.get("days"), Some(&TruthParamValue::Integer(15)));

        let mut out_of_range =
            TruthParamValue::object([("amount", TruthParamValue::Integer(9_007_199_254_740_993))]);
        assert_eq!(
            canonicalize_fee_parameters("FEE_ITEM", &mut out_of_range),
            Err("amount")
        );
    }

    #[test]
    fn missing_proposal_metadata_uses_current_wire_defaults() {
        assert_eq!(
            normalized_proposal_metadata("", ""),
            ("unknown".into(), "direct".into())
        );
        assert_eq!(
            normalized_proposal_metadata("critical", "llm"),
            ("critical".into(), "llm".into())
        );
    }
}
