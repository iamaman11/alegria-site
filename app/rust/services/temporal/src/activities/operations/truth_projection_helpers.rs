// Transport serialization at the Temporal projection boundary. Keep identity
// and Truth decisions separate from wire-format JSON conversion.
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
