use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlDecisionInput {
    pub run_id: String,
    pub step_name: String,
    pub layer_confidence: f32,
    pub completeness_score: f32,
    pub unresolved_mappings: usize,
    pub contradiction_blocked: bool,
    pub conflict_count: usize,
    pub loss_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlDecisionOutput {
    pub requires_hitl: bool,
    pub route: String,
    pub task_type: Option<String>,
    pub priority: i32,
    pub reason: Option<String>,
}

pub fn execute(input: &HitlDecisionInput) -> HitlDecisionOutput {
    if input.contradiction_blocked {
        return HitlDecisionOutput {
            requires_hitl: true,
            route: "hard_block".to_string(),
            task_type: Some("fact_conflict".to_string()),
            priority: 200,
            reason: Some("critical contradiction detected".to_string()),
        };
    }

    if input.conflict_count > 0 || input.loss_count > 0 {
        return HitlDecisionOutput {
            requires_hitl: true,
            route: "queue_hitl".to_string(),
            task_type: Some("quality_review".to_string()),
            priority: 120,
            reason: Some("conflicts/losses require human decision".to_string()),
        };
    }

    if input.layer_confidence < 0.60
        || input.completeness_score < 0.80
        || input.unresolved_mappings > 0
    {
        return HitlDecisionOutput {
            requires_hitl: true,
            route: "queue_hitl".to_string(),
            task_type: Some("low_confidence".to_string()),
            priority: 100,
            reason: Some("low confidence or unresolved mappings".to_string()),
        };
    }

    HitlDecisionOutput {
        requires_hitl: false,
        route: "auto_approve".to_string(),
        task_type: None,
        priority: 0,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contradiction_blocked_routes_to_hard_block() {
        let output = execute(&HitlDecisionInput {
            run_id: "run".to_string(),
            step_name: "draft_qa".to_string(),
            layer_confidence: 0.95,
            completeness_score: 0.95,
            unresolved_mappings: 0,
            contradiction_blocked: true,
            conflict_count: 1,
            loss_count: 0,
        });

        assert!(output.requires_hitl);
        assert_eq!(output.route, "hard_block");
        assert_eq!(output.task_type.as_deref(), Some("fact_conflict"));
        assert_eq!(output.priority, 200);
        assert_eq!(
            output.reason.as_deref(),
            Some("critical contradiction detected")
        );
    }

    #[test]
    fn non_blocking_conflicts_queue_hitl() {
        let output = execute(&HitlDecisionInput {
            run_id: "run".to_string(),
            step_name: "draft_qa".to_string(),
            layer_confidence: 0.95,
            completeness_score: 0.95,
            unresolved_mappings: 0,
            contradiction_blocked: false,
            conflict_count: 1,
            loss_count: 0,
        });

        assert!(output.requires_hitl);
        assert_eq!(output.route, "queue_hitl");
        assert_eq!(output.task_type.as_deref(), Some("quality_review"));
        assert_eq!(output.priority, 120);
    }
}
