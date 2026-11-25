//! Neo4j projection schema and writes.
//!
//! PostgreSQL is the authoritative source of projection rows. Its adapters
//! load those rows and pass typed snapshots here; this module knows nothing
//! about SQLx, PostgreSQL pools, outbox delivery, or workflow orchestration.
//! A change to the graph schema therefore belongs to the graph owner.

use anyhow::{bail, Context, Result};
use neo4rs::{query, Graph};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleInstanceProjection {
    pub rule_instance_id: String,
    pub context_key: String,
    pub rule_type_key: String,
    pub concept_key: String,
    pub role_type: String,
    pub status: String,
}

pub async fn upsert_rule_instance(graph: &Graph, row: RuleInstanceProjection) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (ctx:VisaContext {context_key: $context_key})
                MERGE (c:Concept {concept_key: $concept_key})
                MERGE (r:RuleInstance {rule_instance_id: $rule_instance_id})
                SET r.rule_type_key = $rule_type_key,
                    r.role_type = $role_type,
                    r.status = $status
                MERGE (ctx)-[:HAS_RULE]->(r)
                MERGE (r)-[:CONCERNS]->(c)
                "#,
            )
            .param("context_key", row.context_key)
            .param("concept_key", row.concept_key)
            .param("rule_instance_id", row.rule_instance_id)
            .param("rule_type_key", row.rule_type_key)
            .param("role_type", row.role_type)
            .param("status", row.status),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptProjection {
    pub concept_key: String,
    pub concept_type: String,
    pub label_ru: String,
    pub status: String,
}

pub async fn upsert_concept(graph: &Graph, row: ConceptProjection) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (c:Concept {concept_key: $concept_key})
                SET c.concept_type = $concept_type,
                    c.label_ru = $label_ru,
                    c.status = $status
                "#,
            )
            .param("concept_key", row.concept_key)
            .param("concept_type", row.concept_type)
            .param("label_ru", row.label_ru)
            .param("status", row.status),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageContextProjection {
    pub url_path: String,
    pub context_key: String,
    pub mapping_status: String,
}

pub async fn upsert_page_context(graph: &Graph, row: PageContextProjection) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (p:Page {url_path: $url_path})
                SET p.mapping_status = $mapping_status
                MERGE (ctx:VisaContext {context_key: $context_key})
                MERGE (p)-[:ABOUT_CONTEXT]->(ctx)
                "#,
            )
            .param("url_path", row.url_path)
            .param("mapping_status", row.mapping_status)
            .param("context_key", row.context_key),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordClusterProjection {
    pub cluster_key: String,
    pub scope_signature: String,
    pub seed_keyword: String,
    pub dominant_intent: String,
    pub status: String,
}

pub async fn upsert_keyword_cluster(graph: &Graph, row: KeywordClusterProjection) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (kc:KeywordCluster {cluster_key: $cluster_key})
                SET kc.scope_signature = $scope_signature,
                    kc.seed_keyword = $seed_keyword,
                    kc.dominant_intent = $dominant_intent,
                    kc.status = $status
                MERGE (intent:Intent {intent_key: $dominant_intent})
                MERGE (kc)-[:TARGETS_INTENT]->(intent)
                "#,
            )
            .param("cluster_key", row.cluster_key)
            .param("scope_signature", row.scope_signature)
            .param("seed_keyword", row.seed_keyword)
            .param("dominant_intent", row.dominant_intent)
            .param("status", row.status),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerpPatternProjection {
    pub serp_pattern_key: String,
    pub query_batch_key: String,
    pub scope_signature: String,
    pub query_text: String,
    pub pattern_type: String,
    pub dominant_intent: String,
    pub status: String,
}

pub async fn upsert_serp_pattern(graph: &Graph, row: SerpPatternProjection) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (sp:SERPPattern {serp_pattern_key: $serp_pattern_key})
                SET sp.query_batch_key = $query_batch_key,
                    sp.scope_signature = $scope_signature,
                    sp.query = $query,
                    sp.pattern_type = $pattern_type,
                    sp.dominant_intent = $dominant_intent,
                    sp.status = $status
                MERGE (intent:Intent {intent_key: $dominant_intent})
                MERGE (sp)-[:TARGETS_INTENT]->(intent)
                "#,
            )
            .param("serp_pattern_key", row.serp_pattern_key)
            .param("query_batch_key", row.query_batch_key)
            .param("scope_signature", row.scope_signature)
            .param("query", row.query_text)
            .param("pattern_type", row.pattern_type)
            .param("dominant_intent", row.dominant_intent)
            .param("status", row.status),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBlueprintProjection {
    pub blueprint_key: String,
    pub page_type_key: String,
    pub dominant_intent: String,
    pub scope_class: String,
    pub blueprint_version: i64,
    pub status: String,
}

pub async fn upsert_page_blueprint(graph: &Graph, row: PageBlueprintProjection) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (bp:PageBlueprint {blueprint_key: $blueprint_key})
                SET bp.page_type_key = $page_type_key,
                    bp.dominant_intent = $dominant_intent,
                    bp.scope_class = $scope_class,
                    bp.blueprint_version = $blueprint_version,
                    bp.status = $status
                MERGE (intent:Intent {intent_key: $dominant_intent})
                MERGE (bp)-[:TARGETS_INTENT]->(intent)
                "#,
            )
            .param("blueprint_key", row.blueprint_key)
            .param("page_type_key", row.page_type_key)
            .param("dominant_intent", row.dominant_intent)
            .param("scope_class", row.scope_class)
            .param("blueprint_version", row.blueprint_version)
            .param("status", row.status),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageNodeProjection {
    pub page_node_key: String,
    pub scope_signature: String,
    pub keyword_cluster_key: Option<String>,
    pub blueprint_key: Option<String>,
    pub page_type_key: String,
    pub dominant_intent: String,
    pub canonical_slug: String,
    pub canonical_url_path: String,
    pub lifecycle_state: String,
}

// A PageNode is one authoritative PostgreSQL snapshot. Optional edges must
// move with the snapshot, including when an association is removed. Writing
// it in separate auto-commit statements could leave a new node with the old
// relationships when a later provider request fails.
const PAGE_NODE_SNAPSHOT_CYPHER: &str = r#"
    MERGE (pn:PageNode {page_node_key: $page_node_key})
    SET pn.scope_signature = $scope_signature,
        pn.page_type_key = $page_type_key,
        pn.dominant_intent = $dominant_intent,
        pn.canonical_slug = $canonical_slug,
        pn.canonical_url_path = $canonical_url_path,
        pn.lifecycle_state = $lifecycle_state
    WITH pn
    OPTIONAL MATCH (pn)-[old_intent:TARGETS_INTENT]->(old_i:Intent)
    WHERE old_i.intent_key <> $dominant_intent
    DELETE old_intent
    WITH DISTINCT pn
    MERGE (intent:Intent {intent_key: $dominant_intent})
    MERGE (pn)-[:TARGETS_INTENT]->(intent)
    WITH pn
    OPTIONAL MATCH (old_cluster:KeywordCluster)-[old_proposal:PROPOSES_PAGE]->(pn)
    WHERE $keyword_cluster_key = '' OR old_cluster.cluster_key <> $keyword_cluster_key
    DELETE old_proposal
    WITH DISTINCT pn
    FOREACH (_ IN CASE WHEN $keyword_cluster_key = '' THEN [] ELSE [1] END |
        MERGE (cluster:KeywordCluster {cluster_key: $keyword_cluster_key})
        MERGE (cluster)-[:PROPOSES_PAGE]->(pn)
    )
    WITH pn
    OPTIONAL MATCH (pn)-[old_uses:USES_BLUEPRINT]->(old_bp:PageBlueprint)
    WHERE $blueprint_key = '' OR old_bp.blueprint_key <> $blueprint_key
    DELETE old_uses
    WITH DISTINCT pn
    FOREACH (_ IN CASE WHEN $blueprint_key = '' THEN [] ELSE [1] END |
        MERGE (bp:PageBlueprint {blueprint_key: $blueprint_key})
        MERGE (pn)-[:USES_BLUEPRINT]->(bp)
    )
    "#;

/// The empty-string parameter denotes a missing optional edge only. An
/// explicitly supplied empty key is a contract violation, not a deletion.
fn optional_snapshot_edge_key(key: Option<String>, field: &str) -> Result<String> {
    match key {
        Some(value) if value.trim().is_empty() => bail!("{field} cannot be empty"),
        Some(value) => Ok(value),
        None => Ok(String::new()),
    }
}

pub async fn upsert_page_node(graph: &Graph, row: PageNodeProjection) -> Result<()> {
    let keyword_cluster_key =
        optional_snapshot_edge_key(row.keyword_cluster_key, "keyword_cluster_key")?;
    let blueprint_key = optional_snapshot_edge_key(row.blueprint_key, "blueprint_key")?;
    graph
        .run(
            query(PAGE_NODE_SNAPSHOT_CYPHER)
            .param("page_node_key", row.page_node_key)
            .param("scope_signature", row.scope_signature)
            .param("page_type_key", row.page_type_key)
            .param("dominant_intent", row.dominant_intent)
            .param("canonical_slug", row.canonical_slug)
            .param("canonical_url_path", row.canonical_url_path)
            .param("lifecycle_state", row.lifecycle_state)
            .param("keyword_cluster_key", keyword_cluster_key)
            .param("blueprint_key", blueprint_key),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentGapProjection {
    pub content_gap_key: String,
    pub scope_signature: String,
    pub page_node_key: Option<String>,
    pub missing_topic: String,
    pub severity: String,
    pub status: String,
}

const CONTENT_GAP_SNAPSHOT_CYPHER: &str = r#"
    MERGE (gap:ContentGap {content_gap_key: $content_gap_key})
    SET gap.scope_signature = $scope_signature,
        gap.missing_topic = $missing_topic,
        gap.severity = $severity,
        gap.status = $status
    WITH gap
    OPTIONAL MATCH (old_page:PageNode)-[old_gap:HAS_CONTENT_GAP]->(gap)
    WHERE $page_node_key = '' OR old_page.page_node_key <> $page_node_key
    DELETE old_gap
    WITH DISTINCT gap
    FOREACH (_ IN CASE WHEN $page_node_key = '' THEN [] ELSE [1] END |
        MERGE (pn:PageNode {page_node_key: $page_node_key})
        MERGE (pn)-[:HAS_CONTENT_GAP]->(gap)
    )
    "#;

pub async fn upsert_content_gap(graph: &Graph, row: ContentGapProjection) -> Result<()> {
    let page_node_key = optional_snapshot_edge_key(row.page_node_key, "page_node_key")?;
    graph
        .run(
            query(CONTENT_GAP_SNAPSHOT_CYPHER)
            .param("content_gap_key", row.content_gap_key)
            .param("scope_signature", row.scope_signature)
            .param("missing_topic", row.missing_topic)
            .param("severity", row.severity)
            .param("status", row.status)
            .param("page_node_key", page_node_key),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkRecommendationProjection {
    pub link_recommendation_key: String,
    pub scope_signature: String,
    pub source_page_key: String,
    pub target_page_key: String,
    pub link_role: String,
    pub anchor_strategy: String,
    pub required_flag: bool,
    /// Finite score in [0, 1], persisted as a Neo4j float for graph readers.
    pub score: f64,
    pub status: String,
}

const LINK_RECOMMENDATION_CYPHER: &str = r#"
    MATCH (source:PageNode {page_node_key: $source_page_key})
    MATCH (target:PageNode {page_node_key: $target_page_key})
    WHERE source.scope_signature = $scope_signature
      AND target.scope_signature = $scope_signature
    MERGE (source)-[r:RECOMMENDS_LINK_TO {link_recommendation_key: $link_recommendation_key}]->(target)
    SET r.link_role = $link_role,
        r.anchor_strategy = $anchor_strategy,
        r.required_flag = $required_flag,
        r.score = $score,
        r.status = $status
    RETURN count(r) AS projected_count
    "#;

/// Call only after the authoritative PageNode rows of both endpoints have
/// been materialized. Link events can be claimed before PageNode events.
/// A missing or differently scoped endpoint must fail the Outbox delivery,
/// rather than silently ACK a relationship that Neo4j did not materialize.
pub async fn upsert_link_recommendation(
    graph: &Graph,
    row: LinkRecommendationProjection,
) -> Result<()> {
    if row.scope_signature.trim().is_empty() {
        bail!("link recommendation projection requires a nonempty scope_signature");
    }
    if !row.score.is_finite() || !(0.0..=1.0).contains(&row.score) {
        bail!("link recommendation projection score must be a finite number between 0 and 1");
    }
    let link_recommendation_key = row.link_recommendation_key.clone();
    let mut rows = graph
        .execute(link_recommendation_query(row))
        .await
        .context("execute scoped Neo4j link recommendation projection")?;
    let count = rows
        .next()
        .await
        .context("read scoped Neo4j link recommendation projection result")?
        .context("Neo4j link recommendation projection returned no count")?
        .get::<i64>("projected_count")
        .context("read Neo4j projected_count")?;
    // The stream is fully consumed before the Outbox caller is allowed to ACK.
    if rows
        .next()
        .await
        .context("finish scoped Neo4j link recommendation projection stream")?
        .is_some()
    {
        bail!("link recommendation projection returned more than one result row");
    }
    if count != 1 {
        bail!(
            "link recommendation {link_recommendation_key} expected one same-scope source/target pair, projected {count}"
        );
    }
    Ok(())
}

fn link_recommendation_query(row: LinkRecommendationProjection) -> neo4rs::Query {
    query(LINK_RECOMMENDATION_CYPHER)
        .param("link_recommendation_key", row.link_recommendation_key)
        .param("scope_signature", row.scope_signature)
        .param("source_page_key", row.source_page_key)
        .param("target_page_key", row.target_page_key)
        .param("link_role", row.link_role)
        .param("anchor_strategy", row.anchor_strategy)
        .param("required_flag", row.required_flag)
        .param("score", row.score)
        .param("status", row.status)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CannibalizationConflictProjection {
    pub conflict_key: String,
    pub page_key_a: String,
    pub page_key_b: String,
    pub conflict_reason: String,
    pub severity: String,
    pub status: String,
}

pub async fn upsert_cannibalization_conflict(
    graph: &Graph,
    row: CannibalizationConflictProjection,
) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (a:PageNode {page_node_key: $page_key_a})
                MERGE (b:PageNode {page_node_key: $page_key_b})
                MERGE (a)-[r:CONFLICTS_WITH {conflict_key: $conflict_key}]->(b)
                SET r.conflict_reason = $conflict_reason,
                    r.severity = $severity,
                    r.status = $status
                "#,
            )
            .param("conflict_key", row.conflict_key)
            .param("page_key_a", row.page_key_a)
            .param("page_key_b", row.page_key_b)
            .param("conflict_reason", row.conflict_reason)
            .param("severity", row.severity)
            .param("status", row.status),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBriefProjection {
    pub page_brief_key: String,
    pub page_node_key: String,
    pub blueprint_key: String,
    pub title: String,
    pub meta_description: String,
    pub status: String,
}

pub async fn upsert_page_brief(graph: &Graph, row: PageBriefProjection) -> Result<()> {
    graph
        .run(
            query(
                r#"
                MERGE (brief:PageBrief {page_brief_key: $page_brief_key})
                SET brief.title = $title,
                    brief.meta_description = $meta_description,
                    brief.status = $status
                MERGE (pn:PageNode {page_node_key: $page_node_key})
                MERGE (bp:PageBlueprint {blueprint_key: $blueprint_key})
                MERGE (pn)-[:HAS_BRIEF]->(brief)
                MERGE (brief)-[:USES_BLUEPRINT]->(bp)
                "#,
            )
            .param("page_brief_key", row.page_brief_key)
            .param("page_node_key", row.page_node_key)
            .param("blueprint_key", row.blueprint_key)
            .param("title", row.title)
            .param("meta_description", row.meta_description)
            .param("status", row.status),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod projection_contract_tests {
    use super::*;

    #[test]
    fn optional_snapshot_attachments_are_replaced_in_one_neo4j_statement() {
        assert_eq!(optional_snapshot_edge_key(None, "key").unwrap(), "");
        assert_eq!(
            optional_snapshot_edge_key(Some("key".into()), "key").unwrap(),
            "key"
        );
        assert!(optional_snapshot_edge_key(Some(" ".into()), "key").is_err());
        for (cypher, old_edge, new_edge) in [
            (
                PAGE_NODE_SNAPSHOT_CYPHER,
                "DELETE old_proposal",
                "MERGE (cluster)-[:PROPOSES_PAGE]->(pn)",
            ),
            (
                PAGE_NODE_SNAPSHOT_CYPHER,
                "DELETE old_uses",
                "MERGE (pn)-[:USES_BLUEPRINT]->(bp)",
            ),
            (
                CONTENT_GAP_SNAPSHOT_CYPHER,
                "DELETE old_gap",
                "MERGE (pn)-[:HAS_CONTENT_GAP]->(gap)",
            ),
        ] {
            assert!(cypher.contains(old_edge));
            assert!(cypher.contains(new_edge));
            assert!(cypher.contains("WITH DISTINCT"));
            assert!(cypher.contains("FOREACH"));
        }
    }

    /// The relationship must MATCH both previously materialized PageNodes,
    /// never create bare nodes as a side effect of an out-of-order link event.
    /// Check the actual query template, independent of neo4rs internal APIs.
    #[test]
    fn link_projection_requires_existing_endpoints_and_expected_parameters() {
        use std::collections::BTreeSet;

        assert!(LINK_RECOMMENDATION_CYPHER.contains("MATCH (source:PageNode"));
        assert!(LINK_RECOMMENDATION_CYPHER.contains("MATCH (target:PageNode"));
        assert!(LINK_RECOMMENDATION_CYPHER.contains("source.scope_signature = $scope_signature"));
        assert!(LINK_RECOMMENDATION_CYPHER.contains("target.scope_signature = $scope_signature"));
        assert!(LINK_RECOMMENDATION_CYPHER.contains("MERGE (source)-[r:RECOMMENDS_LINK_TO"));
        assert!(LINK_RECOMMENDATION_CYPHER.contains("RETURN count(r) AS projected_count"));
        let actual: BTreeSet<String> = LINK_RECOMMENDATION_CYPHER
            .split('$')
            .skip(1)
            .map(|suffix| {
                suffix
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect::<String>()
            })
            .collect();
        let expected: BTreeSet<String> = [
            "link_recommendation_key",
            "scope_signature",
            "source_page_key",
            "target_page_key",
            "link_role",
            "anchor_strategy",
            "required_flag",
            "score",
            "status",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(actual, expected);
    }
}
