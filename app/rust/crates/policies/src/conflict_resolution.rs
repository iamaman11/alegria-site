use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    GovernmentWins,
    ConsensusWins,
    Disputed,
}

#[derive(Debug, Clone)]
pub struct SourceRegistryEntry {
    pub source_type: String,
    pub trust_level: i64,
}

#[derive(Debug, Clone)]
pub struct ConflictRuleCandidate {
    pub source_key: Option<String>,
    pub fact_value: Option<String>,
}

pub fn resolve_source_conflict_typed(
    rules: &[ConflictRuleCandidate],
    source_registry: &BTreeMap<String, SourceRegistryEntry>,
) -> ConflictResolution {
    if rules.is_empty() {
        return ConflictResolution::Disputed;
    }

    for rule in rules {
        let Some(source_key) = rule.source_key.as_deref() else {
            continue;
        };
        if let Some(entry) = source_registry.get(source_key) {
            if entry.trust_level >= 9 || entry.source_type.eq_ignore_ascii_case("government") {
                return ConflictResolution::GovernmentWins;
            }
        }
    }

    let mut freq: HashMap<String, usize> = HashMap::new();
    for rule in rules {
        let Some(fact_value) = rule.fact_value.as_ref() else {
            continue;
        };
        *freq.entry(fact_value.clone()).or_insert(0) += 1;
    }

    if freq.values().any(|&n| n >= 2) {
        ConflictResolution::ConsensusWins
    } else {
        ConflictResolution::Disputed
    }
}
