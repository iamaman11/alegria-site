impl RuntimeProtoPayload for String {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.StringPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(StringPayload {
            value: self.clone(),
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        Ok(decode_prost::<StringPayload>(payload_bytes, "StringPayload")?.value)
    }
}

impl RuntimeProtoPayload for &str {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.StringPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(StringPayload {
            value: (*self).to_string(),
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(_payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        Err(contract_violation(
            "cannot decode borrowed &str runtime payload",
        ))
    }
}

impl RuntimeProtoPayload for VerifyReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.VerifyReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "VerifyReport")
    }
}

impl RuntimeProtoPayload for PersistReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.PersistReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "PersistReport")
    }
}

impl RuntimeProtoPayload for HitlPauseInfo {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.HitlPauseInfo"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "HitlPauseInfo")
    }
}

impl RuntimeProtoPayload for HitlResolutionInput {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.HitlResolutionInput"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "HitlResolutionInput")
    }
}

impl RuntimeProtoPayload for HitlDecision {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.HitlDecision"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "HitlDecision")
    }
}

impl RuntimeProtoPayload for ValidationInputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ValidationInputPayload"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ValidationInputPayload")
    }
}

impl RuntimeProtoPayload for ValidationReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ValidationReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ValidationReport")
    }
}

impl RuntimeProtoPayload for FreshnessReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.FreshnessReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "FreshnessReport")
    }
}

impl RuntimeProtoPayload for ReconcileTargetInputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ReconcileTargetInputPayload"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ReconcileTargetInputPayload")
    }
}

impl RuntimeProtoPayload for ProjectionBarrierAuditInputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ProjectionBarrierAuditInputPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ProjectionBarrierAuditInputPayload")
    }
}

impl RuntimeProtoPayload for ProjectionBarrierAuditOutputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ProjectionBarrierAuditOutputPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ProjectionBarrierAuditOutputPayload")
    }
}

impl RuntimeProtoPayload for ExtractedPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ExtractedPayloadState"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        let payload = ExtractedPayloadState {
            rule_instances: self
                .rule_instances
                .iter()
                .map(|rule| RuleInstanceCandidateState {
                    rule_type_key: rule.rule_type_key.clone(),
                    concept_key: rule.concept_key.clone(),
                    role_type: encode_rule_role_type(rule.role_type),
                    params: Some(encode_rule_params_state(&rule.params)),
                    status: rule.status.clone(),
                    source_key: rule.source_key.clone(),
                    condition_expr: rule.condition_expr.clone(),
                })
                .collect(),
            facts: self.facts.iter().map(encode_fact_value_state).collect(),
        };
        Ok(payload.encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload =
            decode_prost::<ExtractedPayloadState>(payload_bytes, "ExtractedPayloadState")?;
        let mut rules = Vec::with_capacity(payload.rule_instances.len());
        for rule in payload.rule_instances {
            let role_type = decode_rule_role_type(rule.role_type)?;
            rules.push(RuleInstanceCandidate {
                rule_type_key: rule.rule_type_key,
                concept_key: rule.concept_key,
                role_type,
                params: decode_rule_params_state(rule.params),
                status: rule.status,
                source_key: rule.source_key,
                condition_expr: rule.condition_expr,
            });
        }
        Ok(ExtractedPayload {
            rule_instances: rules,
            facts: payload
                .facts
                .into_iter()
                .map(decode_fact_value_state)
                .collect(),
        })
    }
}

impl RuntimeProtoPayload for BTreeMap<String, String> {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.GenerationResultState"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(GenerationResultState {
            blocks: self
                .iter()
                .map(|(block_key, html)| GenerationBlockState {
                    block_key: block_key.clone(),
                    html: html.clone(),
                })
                .collect(),
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload =
            decode_prost::<GenerationResultState>(payload_bytes, "GenerationResultState")?;
        Ok(payload
            .blocks
            .into_iter()
            .map(|block| (block.block_key, block.html))
            .collect())
    }
}

impl RuntimeProtoPayload for ReconcileSummary {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ReconcileSummaryPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(ReconcileSummaryPayload {
            open_dlq: self.open_dlq,
            stale_runs: self.stale_runs,
            pending_hitl_runs: self.pending_hitl_runs,
            stuck_steps: self.stuck_steps,
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload =
            decode_prost::<ReconcileSummaryPayload>(payload_bytes, "ReconcileSummaryPayload")?;
        Ok(Self {
            open_dlq: payload.open_dlq,
            stale_runs: payload.stale_runs,
            pending_hitl_runs: payload.pending_hitl_runs,
            stuck_steps: payload.stuck_steps,
        })
    }
}

impl RuntimeProtoPayload for ReconcileTargetReportRecord {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ReconcileTargetReportPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(ReconcileTargetReportPayload {
            target_system: self.target_system.clone(),
            dry_run: self.dry_run,
            stale_candidates: self.stale_candidates,
            failed_candidates: self.failed_candidates,
            reset_stale_processing: self.reset_stale_processing,
            requeued_failed: self.requeued_failed,
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload = decode_prost::<ReconcileTargetReportPayload>(
            payload_bytes,
            "ReconcileTargetReportPayload",
        )?;
        Ok(Self {
            target_system: payload.target_system,
            dry_run: payload.dry_run,
            stale_candidates: payload.stale_candidates,
            failed_candidates: payload.failed_candidates,
            reset_stale_processing: payload.reset_stale_processing,
            requeued_failed: payload.requeued_failed,
        })
    }
}

impl RuntimeProtoPayload for RuntimeErrorPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.RuntimeErrorPayload"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "RuntimeErrorPayload")
    }
}
