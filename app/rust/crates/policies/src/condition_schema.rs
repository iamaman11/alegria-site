use anyhow::{bail, Result};
use contracts::generated::alegria::condition::v1::{scalar_value, ConditionExprV1};

pub fn validate_condition(condition: &ConditionExprV1) -> Result<()> {
    if condition.v != 1 {
        bail!("unsupported condition DSL version");
    }
    match condition.op.as_str() {
        "eq" | "lt" => {
            if condition.field.trim().is_empty() {
                bail!("comparison condition requires a field");
            }
            let value = condition
                .value
                .as_ref()
                .and_then(|scalar| scalar.value.as_ref());
            if value.is_none() || !condition.args.is_empty() {
                bail!("comparison condition requires a scalar value and no child expressions");
            }
            if condition.op == "lt" && !matches!(value, Some(scalar_value::Value::IntValue(_))) {
                bail!("lt condition requires an integer value");
            }
        }
        "and" | "or" => {
            if condition.args.is_empty() {
                bail!("logical condition requires at least one child");
            }
            if !condition.field.is_empty() || condition.value.is_some() {
                bail!("logical condition cannot contain a field or scalar value");
            }
            for child in &condition.args {
                validate_condition(child)?;
            }
        }
        _ => bail!("unsupported condition operation"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::condition::v1::ScalarValue;

    fn less_than(limit: i64) -> ConditionExprV1 {
        ConditionExprV1 {
            v: 1,
            op: "lt".to_string(),
            field: "age".to_string(),
            value: Some(ScalarValue {
                value: Some(scalar_value::Value::IntValue(limit)),
            }),
            ..ConditionExprV1::default()
        }
    }

    #[test]
    fn validates_supported_integer_comparison() {
        assert!(validate_condition(&less_than(18)).is_ok());
    }

    #[test]
    fn rejects_empty_logical_group_and_unsupported_operation() {
        let empty_and = ConditionExprV1 {
            v: 1,
            op: "and".to_string(),
            ..ConditionExprV1::default()
        };
        assert!(validate_condition(&empty_and).is_err());

        let unknown = ConditionExprV1 {
            v: 1,
            op: "always".to_string(),
            ..ConditionExprV1::default()
        };
        assert!(validate_condition(&unknown).is_err());
    }

    #[test]
    fn rejects_incomplete_and_mismatched_comparisons() {
        let mut missing_field = less_than(18);
        missing_field.field.clear();
        assert!(validate_condition(&missing_field).is_err());

        let mut wrong_value = less_than(18);
        wrong_value.value = Some(ScalarValue {
            value: Some(scalar_value::Value::StringValue("18".to_string())),
        });
        assert!(validate_condition(&wrong_value).is_err());

        let mut nested_invalid = ConditionExprV1 {
            v: 1,
            op: "and".to_string(),
            args: vec![less_than(18)],
            ..ConditionExprV1::default()
        };
        assert!(validate_condition(&nested_invalid).is_ok());
        nested_invalid.args[0].v = 2;
        assert!(validate_condition(&nested_invalid).is_err());
    }
}
