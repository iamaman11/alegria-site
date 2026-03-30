//! Deterministic HTML rendering from typed read-model contracts.
//! Runtime path intentionally avoids raw JSON values as the working model.

#[derive(Debug, Clone, Default)]
pub struct CitationRule {
    pub rule_instance_id: String,
    pub rule_type_key: String,
    pub role: RuleRole,
    pub params: RuleParamsView,
    pub status: String,
    pub effective_from: String,
    pub effective_to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RuleRole {
    #[default]
    Unknown,
    DocumentRequired,
    FormRequired,
    FeeItem,
    TimelineItem,
    Generic,
}

#[derive(Debug, Clone, Default)]
pub enum RuleParamsView {
    #[default]
    None,
    Document {
        severity: String,
        subtype: Option<String>,
        notarization_required: bool,
    },
    Fee {
        amount: f64,
        currency: String,
    },
    Timeline {
        days: i64,
    },
}

#[derive(Debug, Clone, Default)]
struct RenderRule {
    rule_type_key: String,
    title: String,
    role: RuleRole,
    amount: Option<f64>,
    currency: String,
    days: Option<i64>,
    notarization_required: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FaqItem {
    pub question: String,
    pub answer: String,
}

fn parse_rule(fact: &CitationRule) -> RenderRule {
    let default_title = fact.rule_type_key.replace('_', " ");
    let (title, amount, currency, days, notarization_required) = match &fact.params {
        RuleParamsView::None => (default_title.clone(), None, "EUR".to_string(), None, false),
        RuleParamsView::Document {
            subtype,
            notarization_required,
            ..
        } => (
            subtype.clone().unwrap_or(default_title.clone()),
            None,
            "EUR".to_string(),
            None,
            *notarization_required,
        ),
        RuleParamsView::Fee { amount, currency } => {
            (default_title.clone(), Some(*amount), currency.clone(), None, false)
        }
        RuleParamsView::Timeline { days } => {
            (default_title.clone(), None, "EUR".to_string(), Some(*days), false)
        }
    };
    RenderRule {
        rule_type_key: fact.rule_type_key.clone(),
        title,
        role: fact.role.clone(),
        amount,
        currency,
        days,
        notarization_required,
    }
}

pub fn render_block(block_key: &str, rules: &[CitationRule], facts: &[FaqItem]) -> String {
    match block_key {
        "fees_table" => render_fees_table(rules),
        "documents_list" => render_documents_list(rules),
        "faq" => render_faq(facts),
        "timeline" => render_timeline(rules),
        _ => render_default(block_key, facts),
    }
}

pub fn plan_blocks(rules: &[CitationRule], facts: &[FaqItem]) -> Vec<String> {
    let parsed: Vec<RenderRule> = rules.iter().map(parse_rule).collect();
    let mut blocks = Vec::new();
    let has_fees = parsed.iter().any(|r| {
        matches!(r.rule_type_key.as_str(), "fee" | "consular_fee") || r.amount.is_some()
    });
    let has_docs = parsed
        .iter()
        .any(|r| matches!(r.role, RuleRole::DocumentRequired | RuleRole::FormRequired));
    let has_timeline = parsed.iter().any(|r| {
        r.days.is_some()
            || r.rule_type_key.contains("duration")
            || r.rule_type_key.contains("timeline")
    });
    if has_fees {
        blocks.push("fees_table".to_string());
    }
    if has_docs {
        blocks.push("documents_list".to_string());
    }
    if has_timeline {
        blocks.push("timeline".to_string());
    }
    if !facts.is_empty() {
        blocks.push("faq".to_string());
    }
    blocks
}

fn render_fees_table(rules: &[CitationRule]) -> String {
    let rows: String = rules
        .iter()
        .map(parse_rule)
        .filter(|r| matches!(r.rule_type_key.as_str(), "fee" | "consular_fee") || r.amount.is_some())
        .map(|r| {
            let label = html_escape(&r.title);
            let amount = r.amount.unwrap_or(0.0);
            let currency = html_escape(&r.currency);
            format!("<tr><td>{label}</td><td>{amount} {currency}</td></tr>")
        })
        .collect();
    format!(
        "<table class='fees-table'><thead><tr><th>Сбор</th><th>Сумма</th></tr></thead><tbody>{rows}</tbody></table>"
    )
}

fn render_documents_list(rules: &[CitationRule]) -> String {
    let items: String = rules
        .iter()
        .map(parse_rule)
        .filter(|r| matches!(r.role, RuleRole::DocumentRequired | RuleRole::FormRequired))
        .map(|r| {
            let label = html_escape(&r.title);
            let notarized = if r.notarization_required { " (нотариально)" } else { "" };
            format!("<li>{label}{notarized}</li>")
        })
        .collect();
    format!("<ul class='documents-list'>{items}</ul>")
}

fn render_faq(facts: &[FaqItem]) -> String {
    let items: String = facts
        .iter()
        .map(|f| {
            let q = html_escape(&f.question);
            let a = html_escape(&f.answer);
            format!("<dt>{q}</dt><dd>{a}</dd>")
        })
        .collect();
    format!("<dl class='faq'>{items}</dl>")
}

fn render_timeline(rules: &[CitationRule]) -> String {
    let items: String = rules
        .iter()
        .map(parse_rule)
        .filter(|r| r.days.is_some() || r.rule_type_key.contains("duration") || r.rule_type_key.contains("timeline"))
        .map(|r| {
            let label = html_escape(&r.title);
            let days = r.days.unwrap_or(0);
            format!("<li><strong>{label}:</strong> {days} дней</li>")
        })
        .collect();
    format!("<ul class='timeline'>{items}</ul>")
}

fn render_default(block_key: &str, facts: &[FaqItem]) -> String {
    let title = block_key.replace('_', " ");
    let body: String = facts
        .iter()
        .map(|f| format!("<p>{}</p>", html_escape(&f.answer)))
        .collect();
    format!("<h2>{title}</h2>{body}")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fee_fact() -> CitationRule {
        CitationRule {
            rule_instance_id: "r1".to_string(),
            rule_type_key: "consular_fee".to_string(),
            role: RuleRole::FeeItem,
            params: RuleParamsView::Fee {
                amount: 160.0,
                currency: "USD".to_string(),
            },
            status: "active".to_string(),
            effective_from: String::new(),
            effective_to: String::new(),
        }
    }

    #[test]
    fn test_render_fees_table() {
        let html = render_fees_table(&[fee_fact()]);
        assert!(html.contains("Visa fee"));
        assert!(html.contains("160"));
        assert!(html.contains("USD"));
    }

    #[test]
    fn test_plan_blocks_empty() {
        assert!(plan_blocks(&[], &[]).is_empty());
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
    }
}
