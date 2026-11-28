//! Neo4j-owned physical identity constraints for projection writers.
//!
//! Cypher MERGE without a uniqueness constraint does not serialize two
//! concurrent providers that create the same logical node or relationship.
//! A duplicate PageNode turns one scoped link recommendation into multiple
//! physical edges. The Outbox must reject that ambiguity, not ACK it.
//!
//! Installation is an explicit operator/bootstrap step, never an implicit
//! mutation during provider dispatch. Every production connection verifies
//! the installed schema before any projection can be attempted. Historical
//! duplicates make installation FAIL; no data is silently consolidated.

use anyhow::{bail, Context, Result};
use neo4rs::{query, Graph};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstraintEntity {
    Node,
    Relationship,
}

#[derive(Debug, Clone, Copy)]
struct ProjectionConstraint {
    name: &'static str,
    entity: ConstraintEntity,
    label_or_type: &'static str,
    property: &'static str,
    ddl: &'static str,
}

// Only constant, reviewed Cypher is accepted as graph schema DDL. These are
// the actual MERGE identities in projection.rs, not proposed site steps.
const PROJECTION_CONSTRAINTS: &[ProjectionConstraint] = &[
    ProjectionConstraint {
        name: "alegria_visacontext_context_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "VisaContext",
        property: "context_key",
        ddl: "CREATE CONSTRAINT alegria_visacontext_context_key_unique IF NOT EXISTS FOR (n:VisaContext) REQUIRE n.context_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_concept_concept_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "Concept",
        property: "concept_key",
        ddl: "CREATE CONSTRAINT alegria_concept_concept_key_unique IF NOT EXISTS FOR (n:Concept) REQUIRE n.concept_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_ruleinstance_rule_instance_id_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "RuleInstance",
        property: "rule_instance_id",
        ddl: "CREATE CONSTRAINT alegria_ruleinstance_rule_instance_id_unique IF NOT EXISTS FOR (n:RuleInstance) REQUIRE n.rule_instance_id IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_page_url_path_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "Page",
        property: "url_path",
        ddl: "CREATE CONSTRAINT alegria_page_url_path_unique IF NOT EXISTS FOR (n:Page) REQUIRE n.url_path IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_keywordcluster_cluster_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "KeywordCluster",
        property: "cluster_key",
        ddl: "CREATE CONSTRAINT alegria_keywordcluster_cluster_key_unique IF NOT EXISTS FOR (n:KeywordCluster) REQUIRE n.cluster_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_intent_intent_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "Intent",
        property: "intent_key",
        ddl: "CREATE CONSTRAINT alegria_intent_intent_key_unique IF NOT EXISTS FOR (n:Intent) REQUIRE n.intent_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_serppattern_serp_pattern_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "SERPPattern",
        property: "serp_pattern_key",
        ddl: "CREATE CONSTRAINT alegria_serppattern_serp_pattern_key_unique IF NOT EXISTS FOR (n:SERPPattern) REQUIRE n.serp_pattern_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_pageblueprint_blueprint_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "PageBlueprint",
        property: "blueprint_key",
        ddl: "CREATE CONSTRAINT alegria_pageblueprint_blueprint_key_unique IF NOT EXISTS FOR (n:PageBlueprint) REQUIRE n.blueprint_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_pagenode_page_node_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "PageNode",
        property: "page_node_key",
        ddl: "CREATE CONSTRAINT alegria_pagenode_page_node_key_unique IF NOT EXISTS FOR (n:PageNode) REQUIRE n.page_node_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_contentgap_content_gap_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "ContentGap",
        property: "content_gap_key",
        ddl: "CREATE CONSTRAINT alegria_contentgap_content_gap_key_unique IF NOT EXISTS FOR (n:ContentGap) REQUIRE n.content_gap_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_pagebrief_page_brief_key_unique",
        entity: ConstraintEntity::Node,
        label_or_type: "PageBrief",
        property: "page_brief_key",
        ddl: "CREATE CONSTRAINT alegria_pagebrief_page_brief_key_unique IF NOT EXISTS FOR (n:PageBrief) REQUIRE n.page_brief_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_recommends_link_to_link_recommendation_key_unique",
        entity: ConstraintEntity::Relationship,
        label_or_type: "RECOMMENDS_LINK_TO",
        property: "link_recommendation_key",
        ddl: "CREATE CONSTRAINT alegria_recommends_link_to_link_recommendation_key_unique IF NOT EXISTS FOR ()-[r:RECOMMENDS_LINK_TO]-() REQUIRE r.link_recommendation_key IS UNIQUE",
    },
    ProjectionConstraint {
        name: "alegria_conflicts_with_conflict_key_unique",
        entity: ConstraintEntity::Relationship,
        label_or_type: "CONFLICTS_WITH",
        property: "conflict_key",
        ddl: "CREATE CONSTRAINT alegria_conflicts_with_conflict_key_unique IF NOT EXISTS FOR ()-[r:CONFLICTS_WITH]-() REQUIRE r.conflict_key IS UNIQUE",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledConstraint {
    kind: String,
    entity: String,
    labels_or_types: Vec<String>,
    properties: Vec<String>,
}

fn matches_constraint(spec: &ProjectionConstraint, found: &InstalledConstraint) -> bool {
    let (expected_entity, expected_kinds): (&str, &[&str]) = match spec.entity {
        // Neo4j 5 / Cypher 5 report UNIQUENESS; Cypher 25 reports the
        // explicit NODE_PROPERTY_UNIQUENESS type.
        ConstraintEntity::Node => ("NODE", &["UNIQUENESS", "NODE_PROPERTY_UNIQUENESS"]),
        ConstraintEntity::Relationship => (
            "RELATIONSHIP",
            &[
                "RELATIONSHIP_UNIQUENESS",
                "RELATIONSHIP_PROPERTY_UNIQUENESS",
            ],
        ),
    };
    found.entity == expected_entity
        && expected_kinds.contains(&found.kind.as_str())
        && found.labels_or_types.len() == 1
        && found.labels_or_types[0] == spec.label_or_type
        && found.properties.len() == 1
        && found.properties[0] == spec.property
}

/// Operator/bootstrap-only schema installation.
///
/// If historical duplicate nodes or edges exist, Neo4j rejects the relevant
/// constraint. The caller must stop the release and investigate; there is NO
/// automatic cleanup, no timestamp-based winner and no retrying stale writes.
pub async fn install_projection_schema(graph: &Graph) -> Result<()> {
    for spec in PROJECTION_CONSTRAINTS {
        graph
            .run(query(spec.ddl))
            .await
            .with_context(|| format!("install Neo4j identity constraint {}", spec.name))?;
    }
    verify_projection_schema(graph).await
}

/// Read-only fail-closed gate for every production provider connection.
///
/// SHOW is deliberately executed even when another process may have checked
/// the same endpoint: an in-memory OnceCell would not survive a DB restart or
/// detect changed schema. No DDL executes in the effectful request path.
pub async fn verify_projection_schema(graph: &Graph) -> Result<()> {
    let mut rows = graph
        .execute(query(
            "SHOW CONSTRAINTS YIELD name, type, entityType, labelsOrTypes, properties \
             RETURN name, type, entityType, labelsOrTypes, properties",
        ))
        .await
        .context("read Neo4j projection constraint catalog")?;
    let mut installed = BTreeMap::<String, InstalledConstraint>::new();
    while let Some(row) = rows
        .next()
        .await
        .context("read next Neo4j projection constraint row")?
    {
        let name: String = row.get("name").context("decode Neo4j constraint name")?;
        let constraint = InstalledConstraint {
            kind: row.get("type").context("decode Neo4j constraint type")?,
            entity: row
                .get("entityType")
                .context("decode Neo4j constraint entity type")?,
            labels_or_types: row
                .get("labelsOrTypes")
                .context("decode Neo4j constraint labels/types")?,
            properties: row
                .get("properties")
                .context("decode Neo4j constraint properties")?,
        };
        installed.insert(name, constraint);
    }
    for spec in PROJECTION_CONSTRAINTS {
        match installed.get(spec.name) {
            Some(found) if matches_constraint(spec, found) => {}
            Some(found) => {
                bail!(
                    "Neo4j projection identity constraint {} targets wrong schema: {:?}",
                    spec.name,
                    found
                )
            }
            None => {
                bail!(
                    "Neo4j projection identity constraint {} missing; operator must install schema before materialization",
                    spec.name
                )
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_merge_identity_has_an_explicit_named_unique_constraint() {
        assert_eq!(PROJECTION_CONSTRAINTS.len(), 13);
        let names = PROJECTION_CONSTRAINTS
            .iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), PROJECTION_CONSTRAINTS.len());
        for spec in PROJECTION_CONSTRAINTS {
            assert!(spec.ddl.contains("CREATE CONSTRAINT"));
            assert!(spec.ddl.contains(spec.name));
            assert!(spec.ddl.contains("IF NOT EXISTS"));
            assert!(spec.ddl.contains("IS UNIQUE"));
            assert!(spec.ddl.contains(spec.label_or_type));
            assert!(spec.ddl.contains(spec.property));
        }
    }

    #[test]
    fn matching_name_alone_does_not_certify_target_or_constraint_type() {
        let spec = &PROJECTION_CONSTRAINTS[8]; // PageNode key
        let expected = InstalledConstraint {
            kind: "NODE_PROPERTY_UNIQUENESS".to_owned(),
            entity: "NODE".to_owned(),
            labels_or_types: vec!["PageNode".to_owned()],
            properties: vec!["page_node_key".to_owned()],
        };
        assert!(matches_constraint(spec, &expected));
        assert!(matches_constraint(
            spec,
            &InstalledConstraint {
                kind: "UNIQUENESS".to_owned(),
                ..expected.clone()
            }
        ));
        assert!(!matches_constraint(
            spec,
            &InstalledConstraint {
                properties: vec!["source_page_key".to_owned()],
                ..expected.clone()
            }
        ));
        assert!(!matches_constraint(
            spec,
            &InstalledConstraint {
                entity: "RELATIONSHIP".to_owned(),
                ..expected
            }
        ));
    }

    #[test]
    fn keyed_relationships_require_relationship_uniqueness_on_the_exact_type() {
        for spec in PROJECTION_CONSTRAINTS
            .iter()
            .filter(|spec| spec.entity == ConstraintEntity::Relationship)
        {
            let expected = InstalledConstraint {
                kind: "RELATIONSHIP_PROPERTY_UNIQUENESS".to_owned(),
                entity: "RELATIONSHIP".to_owned(),
                labels_or_types: vec![spec.label_or_type.to_owned()],
                properties: vec![spec.property.to_owned()],
            };
            assert!(matches_constraint(spec, &expected));
            assert!(matches_constraint(
                spec,
                &InstalledConstraint {
                    kind: "RELATIONSHIP_UNIQUENESS".to_owned(),
                    ..expected.clone()
                }
            ));
            assert!(!matches_constraint(
                spec,
                &InstalledConstraint {
                    kind: "RELATIONSHIP_KEY".to_owned(),
                    ..expected.clone()
                }
            ));
            assert!(!matches_constraint(
                spec,
                &InstalledConstraint {
                    labels_or_types: vec!["DIFFERENT_RELATIONSHIP".to_owned()],
                    ..expected
                }
            ));
        }
    }
}
