use anyhow::{bail, Result};
use contracts::generated::alegria::condition::v1::ConditionExprV1;

pub fn validate_condition(condition: &ConditionExprV1) -> Result<()> {
    if condition.v != 1 {
        bail!("unsupported condition DSL version");
    }
    if condition.op.trim().is_empty() {
        bail!("condition op is required");
    }
    Ok(())
}
