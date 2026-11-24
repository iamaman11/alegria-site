//! Neo4j-owned read model for planning relationships.
//!
//! The provider returns typed link evidence only. PostgreSQL context joins,
//! canonical identity, policy and confidence merging belong to the caller.

use anyhow::{Context, Result};
use neo4rs::{query, Graph};

#[derive(Debug, Clone, PartialEq)]
pub struct GraphLinkEvidence {
    pub source_page_key: String,
    pub target_page_key: String,
    pub link_role: String,
    pub score: f64,
}

const PLANNING_LINKS_CYPHER: &str = r#"
    MATCH (source:PageNode {scope_signature: $scope_signature})
      -[r:RECOMMENDS_LINK_TO]->
      (target:PageNode {scope_signature: $scope_signature})
    RETURN
      source.page_node_key AS source_page_key,
      target.page_node_key AS target_page_key,
      coalesce(r.link_role, 'semantic_contextual') AS link_role,
      coalesce(r.score, 0.0) AS score
    LIMIT 256
"#;

/// Read the bounded Neo4j neighborhood for a planning scope.
///
/// Malformed endpoint rows are ignored and missing optional properties use
/// the existing defaults. A failed Bolt row stream is *not* a successful
/// partial result: letting the error propagate prevents silently omitting
/// relevant links from a page plan.
pub async fn load_planning_links(
    graph: &Graph,
    scope_signature: &str,
) -> Result<Vec<GraphLinkEvidence>> {
    let mut result = graph
        .execute(
            query(PLANNING_LINKS_CYPHER).param("scope_signature", scope_signature.to_string()),
        )
        .await
        .context("execute planning links Neo4j query")?;
    let mut links = Vec::new();
    while let Some(row) = result
        .next()
        .await
        .context("read planning links Neo4j row stream")?
    {
        let source_page_key = row.get::<String>("source_page_key").unwrap_or_default();
        let target_page_key = row.get::<String>("target_page_key").unwrap_or_default();
        if source_page_key.is_empty() || target_page_key.is_empty() {
            continue;
        }
        links.push(GraphLinkEvidence {
            source_page_key,
            target_page_key,
            link_role: row
                .get::<String>("link_role")
                .unwrap_or_else(|_| "semantic_contextual".to_string()),
            score: row.get::<f64>("score").unwrap_or(0.0),
        });
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn cypher_reads_only_bounded_scope_links_and_uses_stable_defaults() {
        assert!(PLANNING_LINKS_CYPHER.contains("MATCH (source:PageNode"));
        assert!(PLANNING_LINKS_CYPHER.contains("(target:PageNode"));
        assert!(PLANNING_LINKS_CYPHER.contains("-[r:RECOMMENDS_LINK_TO]->"));
        assert!(PLANNING_LINKS_CYPHER.contains("coalesce(r.link_role, 'semantic_contextual')"));
        assert!(PLANNING_LINKS_CYPHER.contains("coalesce(r.score, 0.0)"));
        assert!(PLANNING_LINKS_CYPHER.contains("LIMIT 256"));
        let parameters: BTreeSet<&str> = PLANNING_LINKS_CYPHER
            .split('$')
            .skip(1)
            .map(|suffix| {
                let length = suffix
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                &suffix[..length]
            })
            .collect();
        assert_eq!(parameters, BTreeSet::from(["scope_signature"]));
    }
}
