// Transport conversion stays with the SQL/JSON adapter. Truth domain owns the
// typed params and adjudication rules, never serde_json wire representation.
fn truth_param_value_from_json(value: &Value) -> TruthParamValue {
    match value {
        Value::Null => TruthParamValue::Null,
        Value::Bool(value) => TruthParamValue::Bool(*value),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                TruthParamValue::Integer(integer)
            } else if let Some(decimal) = value.as_f64() {
                TruthParamValue::Decimal(decimal)
            } else {
                TruthParamValue::Null
            }
        }
        Value::String(value) => TruthParamValue::Text(value.clone()),
        Value::Array(values) => TruthParamValue::List(
            values.iter().map(truth_param_value_from_json).collect(),
        ),
        Value::Object(values) => TruthParamValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_from_json(value)))
                .collect(),
        ),
    }
}

fn truth_param_value_to_json(value: &TruthParamValue) -> Value {
    match value {
        TruthParamValue::Null => Value::Null,
        TruthParamValue::Bool(value) => Value::Bool(*value),
        TruthParamValue::Integer(value) => json!(value),
        TruthParamValue::Decimal(value) => json!(value),
        TruthParamValue::Text(value) => Value::String(value.clone()),
        TruthParamValue::List(values) => {
            Value::Array(values.iter().map(truth_param_value_to_json).collect())
        }
        TruthParamValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod truth_param_json_tests {
    use super::*;

    #[test]
    fn persisted_truth_params_preserve_nested_json_wire_values() {
        let value = json!({
            "applicant_profile": "student",
            "fee": [35, 35.25, true, null, {"currency": "EUR"}]
        });
        assert_eq!(truth_param_value_to_json(&truth_param_value_from_json(&value)), value);
        assert_eq!(
            truth_param_value_to_json(&truth_param_value_from_json(&Value::Null)),
            Value::Null
        );
    }
}
