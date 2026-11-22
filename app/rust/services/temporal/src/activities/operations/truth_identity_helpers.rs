fn normalize_numeric_token_fragments(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .flat_map(|token| {
            let normalized = token.replace(',', ".");
            let fragments = normalized
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .filter(|fragment| !fragment.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if fragments.is_empty() {
                vec![normalized]
            } else {
                fragments
            }
        })
        .collect()
}

fn truth_param_value_to_json_local(value: &TruthParamValue) -> Value {
    match value {
        TruthParamValue::Null => Value::Null,
        TruthParamValue::Bool(value) => Value::Bool(*value),
        TruthParamValue::Integer(value) => serde_json::to_value(value).unwrap_or(Value::Null),
        TruthParamValue::Decimal(value) => serde_json::to_value(value).unwrap_or(Value::Null),
        TruthParamValue::Text(value) => Value::String(value.clone()),
        TruthParamValue::List(values) => {
            Value::Array(values.iter().map(truth_param_value_to_json_local).collect())
        }
        TruthParamValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_to_json_local(value)))
                .collect(),
        ),
    }
}

fn semantic_rule_instance_id_local(
    context_key: &str,
    role: &str,
    concept_canonical_key: &str,
) -> String {
    blake3_hex(format!("{context_key}|{role}|{concept_canonical_key}").as_bytes())
}

fn semantic_rule_instance_id_for_scope_local(
    context_key: &str,
    role: &str,
    concept_canonical_key: &str,
    scope: &TruthParamValue,
) -> String {
    if matches!(
        truth_domain::truth_candidates::truth_scope_target(scope),
        Some(truth_domain::truth_candidates::TruthScopeTarget::Global)
    ) {
        // Preserve the old global ID for historical data and projections.
        return semantic_rule_instance_id_local(context_key, role, concept_canonical_key);
    }
    blake3_hex(
        format!(
            "{context_key}|{role}|{concept_canonical_key}|scope:{}",
            scope.stable_signature()
        )
        .as_bytes(),
    )
}

#[cfg(test)]
mod truth_scope_semantic_id_tests {
    use super::*;

    #[test]
    fn scoped_semantic_identity_does_not_collide_with_global_or_other_profiles() {
        let global = TruthParamValue::object(Vec::<(String, TruthParamValue)>::new());
        let student = TruthParamValue::object([(
            "applicant_profile",
            TruthParamValue::Text("student".to_string()),
        )]);
        let minor = TruthParamValue::object([(
            "applicant_profile",
            TruthParamValue::Text("minor".to_string()),
        )]);
        let global_id = semantic_rule_instance_id_for_scope_local("ES|tourist||BY", "FEE_ITEM", "fee", &global);
        assert_eq!(global_id, semantic_rule_instance_id_local("ES|tourist||BY", "FEE_ITEM", "fee"));
        let student_id = semantic_rule_instance_id_for_scope_local("ES|tourist||BY", "FEE_ITEM", "fee", &student);
        let minor_id = semantic_rule_instance_id_for_scope_local("ES|tourist||BY", "FEE_ITEM", "fee", &minor);
        assert_ne!(student_id, global_id);
        assert_ne!(student_id, minor_id);
        assert_ne!(minor_id, global_id);
    }
}
