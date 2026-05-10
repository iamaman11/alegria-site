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
    match condition.op.as_str() {
        "eq" => Ok(scalar_equals(
            &condition.value,
            context.get(&condition.field),
        )),
        "lt" => {
            let lhs = context_i64(context, &condition.field).unwrap_or_default();
            let rhs = scalar_i64(&condition.value).unwrap_or_default();
            Ok(lhs < rhs)
        }
        "and" => {
            for arg in &condition.args {
                if !evaluate_condition(arg, context)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        "or" => {
            for arg in &condition.args {
                if evaluate_condition(arg, context)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}
