use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlOperatorSurfaceInput {
    pub task_type: String,
    pub status: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlOperatorSurfaceOutput {
    pub surface_kind: String,
    pub allowed_actions: Vec<String>,
    pub registry_extension_allowed: bool,
    pub blocked: bool,
}

fn base_actions() -> Vec<String> {
    vec![
        "list".to_string(),
        "show".to_string(),
        "approve".to_string(),
        "reject".to_string(),
        "request_changes".to_string(),
    ]
}

pub fn execute(input: &HitlOperatorSurfaceInput) -> HitlOperatorSurfaceOutput {
    let mut allowed_actions = base_actions();
    let task_type = input.task_type.to_ascii_lowercase();
    let reason = input.reason.to_ascii_lowercase();
    let open = input.status == "pending";
    let registry_extension_allowed = task_type.contains("registry_extension")
        || reason.contains("missing_required_rule_type")
        || reason.contains("coverage_gap")
        || reason.contains("rule_type");

    if !open {
        allowed_actions.retain(|action| action == "list" || action == "show");
    }
    if registry_extension_allowed {
        allowed_actions.push("registry_extension".to_string());
    }
    allowed_actions.sort();
    allowed_actions.dedup();

    HitlOperatorSurfaceOutput {
        surface_kind: if registry_extension_allowed {
            "registry_extension_or_review".to_string()
        } else {
            "review".to_string()
        },
        allowed_actions,
        registry_extension_allowed,
        blocked: !open,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_exposes_full_review_actions() {
        let output = execute(&HitlOperatorSurfaceInput {
            task_type: "quality_review".to_string(),
            status: "pending".to_string(),
            reason: "conflicts/losses require human decision".to_string(),
        });

        assert_eq!(output.surface_kind, "review");
        assert!(output.allowed_actions.contains(&"approve".to_string()));
        assert!(output.allowed_actions.contains(&"reject".to_string()));
        assert!(output
            .allowed_actions
            .contains(&"request_changes".to_string()));
        assert!(!output.registry_extension_allowed);
        assert!(!output.blocked);
    }

    #[test]
    fn registry_extension_task_exposes_extension_action() {
        let output = execute(&HitlOperatorSurfaceInput {
            task_type: "registry_extension".to_string(),
            status: "pending".to_string(),
            reason: "missing_required_rule_type".to_string(),
        });

        assert!(output.registry_extension_allowed);
        assert!(output
            .allowed_actions
            .contains(&"registry_extension".to_string()));
        assert_eq!(output.surface_kind, "registry_extension_or_review");
    }

    #[test]
    fn resolved_task_is_list_show_only() {
        let output = execute(&HitlOperatorSurfaceInput {
            task_type: "quality_review".to_string(),
            status: "resolved".to_string(),
            reason: "done".to_string(),
        });

        assert!(output.blocked);
        assert_eq!(
            output.allowed_actions,
            vec!["list".to_string(), "show".to_string()]
        );
    }
}
