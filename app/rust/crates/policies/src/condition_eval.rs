use std::collections::BTreeMap;

use contracts::generated::alegria::condition::v1::{scalar_value, ConditionExprV1, ScalarValue};
use primitives::errors::DomainError;

#[derive(Debug, Clone, PartialEq)]
pub enum ConditionScalar {
    String(String),
    Int(i64),
    Double(f64),
    Bool(bool),
}

pub type ConditionContext = BTreeMap<String, ConditionScalar>;

fn scalar_equals(value: &Option<ScalarValue>, ctx: Option<&ConditionScalar>) -> bool {
    match (value, ctx) {
        (Some(v), Some(actual)) => match &v.value {
            Some(scalar_value::Value::StringValue(expected)) => {
                matches!(actual, ConditionScalar::String(v) if v == expected)
            }
            Some(scalar_value::Value::IntValue(expected)) => {
                matches!(actual, ConditionScalar::Int(v) if v == expected)
            }
            Some(scalar_value::Value::DoubleValue(expected)) => {
                matches!(actual, ConditionScalar::Double(v) if v == expected)
            }
            Some(scalar_value::Value::BoolValue(expected)) => {
                matches!(actual, ConditionScalar::Bool(v) if v == expected)
            }
            None => false,
        },
        _ => false,
    }
}

fn scalar_i64(value: &Option<ScalarValue>) -> Option<i64> {
    match value.as_ref()?.value.as_ref()? {
        scalar_value::Value::IntValue(v) => Some(*v),
        _ => None,
    }
}

fn context_i64(context: &ConditionContext, field: &str) -> Option<i64> {
    match context.get(field) {
        Some(ConditionScalar::Int(v)) => Some(*v),
        _ => None,
    }
}

pub fn evaluate_condition(
    condition: &ConditionExprV1,
    context: &ConditionContext,
) -> std::result::Result<bool, DomainError> {
    // The evaluator is an applicability gate: malformed or incomplete
    // predicates must never be considered true.
    crate::condition_schema::validate_condition(condition).map_err(|err| {
        DomainError::ValidationFailure {
            message: format!("invalid applicability condition: {err}"),
        }
    })?;
    evaluate_valid_condition(condition, context)
}

fn evaluate_valid_condition(
    condition: &ConditionExprV1,
    context: &ConditionContext,
) -> std::result::Result<bool, DomainError> {
    match condition.op.as_str() {
        "eq" => Ok(scalar_equals(
            &condition.value,
            context.get(&condition.field),
        )),
        "lt" => Ok(
            match (
                context_i64(context, &condition.field),
                scalar_i64(&condition.value),
            ) {
                (Some(actual), Some(bound)) => actual < bound,
                _ => false,
            },
        ),
        "and" => {
            for arg in &condition.args {
                if !evaluate_valid_condition(arg, context)? {
                    return Ok(false);
                }
            }
            Ok(!condition.args.is_empty())
        }
        "or" => {
            for arg in &condition.args {
                if evaluate_valid_condition(arg, context)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(value: scalar_value::Value) -> Option<ScalarValue> {
        Some(ScalarValue { value: Some(value) })
    }

    fn eq(field: &str, value: &str) -> ConditionExprV1 {
        ConditionExprV1 {
            v: 1,
            op: "eq".to_string(),
            field: field.to_string(),
            value: scalar(scalar_value::Value::StringValue(value.to_string())),
            ..ConditionExprV1::default()
        }
    }

    fn lt_age(limit: i64) -> ConditionExprV1 {
        ConditionExprV1 {
            v: 1,
            op: "lt".to_string(),
            field: "age".to_string(),
            value: scalar(scalar_value::Value::IntValue(limit)),
            ..ConditionExprV1::default()
        }
    }

    #[test]
    fn missing_and_wrongly_typed_numeric_context_never_matches() {
        let condition = lt_age(18);
        assert!(!evaluate_condition(&condition, &ConditionContext::new()).unwrap());

        let wrong_type = ConditionContext::from([(
            "age".to_string(),
            ConditionScalar::String("16".to_string()),
        )]);
        assert!(!evaluate_condition(&condition, &wrong_type).unwrap());

        let valid = ConditionContext::from([("age".to_string(), ConditionScalar::Int(16))]);
        assert!(evaluate_condition(&condition, &valid).unwrap());
        let boundary = ConditionContext::from([("age".to_string(), ConditionScalar::Int(18))]);
        assert!(!evaluate_condition(&condition, &boundary).unwrap());
    }

    #[test]
    fn composite_scope_requires_all_applicant_dimensions() {
        let condition = ConditionExprV1 {
            v: 1,
            op: "and".to_string(),
            args: vec![
                eq("citizenship_code", "BY"),
                eq("applicant_type", "student"),
                lt_age(18),
            ],
            ..ConditionExprV1::default()
        };
        let base = ConditionContext::from([
            (
                "citizenship_code".to_string(),
                ConditionScalar::String("BY".to_string()),
            ),
            ("age".to_string(), ConditionScalar::Int(17)),
        ]);
        assert!(!evaluate_condition(&condition, &base).unwrap());

        let mut valid = base;
        valid.insert(
            "applicant_type".to_string(),
            ConditionScalar::String("student".to_string()),
        );
        assert!(evaluate_condition(&condition, &valid).unwrap());

        valid.insert(
            "citizenship_code".to_string(),
            ConditionScalar::String("RU".to_string()),
        );
        assert!(!evaluate_condition(&condition, &valid).unwrap());
    }

    #[test]
    fn empty_logical_group_and_unsupported_version_fail_closed() {
        let condition = ConditionExprV1 {
            v: 1,
            op: "and".to_string(),
            ..ConditionExprV1::default()
        };
        assert!(evaluate_condition(&condition, &ConditionContext::new()).is_err());

        let mut unsupported = lt_age(18);
        unsupported.v = 2;
        assert!(evaluate_condition(&unsupported, &ConditionContext::new()).is_err());
    }
}
