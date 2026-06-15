fn encode_rule_params_state(params: &RuntimeRuleParams) -> RuleParamsState {
    match params {
        RuntimeRuleParams::Fee {
            amount, currency, ..
        } => RuleParamsState {
            amount: Some(*amount),
            currency: Some(currency.clone()),
            days: None,
            extra_json_utf8: Default::default(),
        },
        RuntimeRuleParams::Timeline { days, .. } => RuleParamsState {
            amount: None,
            currency: None,
            days: Some(*days),
            extra_json_utf8: Default::default(),
        },
        _ => RuleParamsState {
            amount: None,
            currency: None,
            days: None,
            extra_json_utf8: Default::default(),
        },
    }
}

fn decode_rule_params_state(params: Option<RuleParamsState>) -> RuntimeRuleParams {
    let Some(params) = params else {
        return RuntimeRuleParams::None;
    };
    if let Some(amount) = params.amount {
        return RuntimeRuleParams::Fee {
            amount,
            currency: params.currency.unwrap_or_else(|| "EUR".to_string()),
            severity: "unspecified".to_string(),
            channel: None,
            conditions_key: String::new(),
        };
    }
    if let Some(days) = params.days {
        return RuntimeRuleParams::Timeline {
            days,
            subtype: None,
            severity: "unspecified".to_string(),
            conditions_key: String::new(),
        };
    }
    RuntimeRuleParams::None
}

fn encode_rule_role_type(role: RuleRoleType) -> i32 {
    match role {
        RuleRoleType::MustProvide => RuleRoleTypeV1::MustProvide as i32,
        RuleRoleType::MustPay => RuleRoleTypeV1::MustPay as i32,
        RuleRoleType::MustSatisfy => RuleRoleTypeV1::MustSatisfy as i32,
        RuleRoleType::Allows => RuleRoleTypeV1::Allows as i32,
        RuleRoleType::Forbids => RuleRoleTypeV1::Forbids as i32,
        RuleRoleType::Timeline => RuleRoleTypeV1::Timeline as i32,
        RuleRoleType::DocumentRequired => RuleRoleTypeV1::DocumentRequired as i32,
        RuleRoleType::EligibilityRule => RuleRoleTypeV1::EligibilityRule as i32,
        RuleRoleType::FeeItem => RuleRoleTypeV1::FeeItem as i32,
        RuleRoleType::TimelineItem => RuleRoleTypeV1::TimelineItem as i32,
        RuleRoleType::WhereToApply => RuleRoleTypeV1::WhereToApply as i32,
        RuleRoleType::AppointmentRule => RuleRoleTypeV1::AppointmentRule as i32,
        RuleRoleType::FormRequired => RuleRoleTypeV1::FormRequired as i32,
        RuleRoleType::Step => RuleRoleTypeV1::Step as i32,
    }
}

fn decode_rule_role_type(value: i32) -> std::result::Result<RuleRoleType, DomainError> {
    let role = RuleRoleTypeV1::try_from(value)
        .map_err(|_| contract_violation(format!("unknown RuleRoleTypeV1 enum value: {value}")))?;
    Ok(match role {
        RuleRoleTypeV1::MustProvide => RuleRoleType::MustProvide,
        RuleRoleTypeV1::MustPay => RuleRoleType::MustPay,
        RuleRoleTypeV1::MustSatisfy => RuleRoleType::MustSatisfy,
        RuleRoleTypeV1::Allows => RuleRoleType::Allows,
        RuleRoleTypeV1::Forbids => RuleRoleType::Forbids,
        RuleRoleTypeV1::Timeline => RuleRoleType::Timeline,
        RuleRoleTypeV1::DocumentRequired => RuleRoleType::DocumentRequired,
        RuleRoleTypeV1::EligibilityRule => RuleRoleType::EligibilityRule,
        RuleRoleTypeV1::FeeItem => RuleRoleType::FeeItem,
        RuleRoleTypeV1::TimelineItem => RuleRoleType::TimelineItem,
        RuleRoleTypeV1::WhereToApply => RuleRoleType::WhereToApply,
        RuleRoleTypeV1::AppointmentRule => RuleRoleType::AppointmentRule,
        RuleRoleTypeV1::FormRequired => RuleRoleType::FormRequired,
        RuleRoleTypeV1::Step => RuleRoleType::Step,
        RuleRoleTypeV1::Unspecified => {
            return Err(contract_violation(
                "unspecified RuleRoleTypeV1 is not allowed",
            ));
        }
    })
}

fn encode_fact_value_state(value: &FactCandidateValue) -> FactValueState {
    match value {
        FactCandidateValue::Null => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::NullValue(
                    true,
                ),
            ),
        },
        FactCandidateValue::Integer(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::IntegerValue(
                    *v,
                ),
            ),
        },
        FactCandidateValue::Decimal(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::DecimalValue(
                    *v,
                ),
            ),
        },
        FactCandidateValue::Text(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::TextValue(
                    v.clone(),
                ),
            ),
        },
        FactCandidateValue::Boolean(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::BooleanValue(
                    *v,
                ),
            ),
        },
    }
}

fn decode_fact_value_state(state: FactValueState) -> FactCandidateValue {
    use contracts::generated::alegria::temporal::v1::fact_value_state::Value;
    match state.value {
        Some(Value::IntegerValue(v)) => FactCandidateValue::Integer(v),
        Some(Value::DecimalValue(v)) => FactCandidateValue::Decimal(v),
        Some(Value::TextValue(v)) => FactCandidateValue::Text(v),
        Some(Value::BooleanValue(v)) => FactCandidateValue::Boolean(v),
        Some(Value::NullValue(_)) | None => FactCandidateValue::Null,
    }
}
