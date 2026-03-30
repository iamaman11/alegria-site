use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ParsedRuleParams {
    #[serde(default)]
    pub amount: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub role_type: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

pub fn parse_rule_params_json(params_json: &[u8]) -> ParsedRuleParams {
    serde_json::from_slice::<ParsedRuleParams>(params_json).unwrap_or_default()
}
