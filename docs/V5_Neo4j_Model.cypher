// Reference graph/domain model. Operational runtime/source-of-truth remains in live Rust code, app/db/schema.sql and automation gates.

// V5_Neo4j_Model.cypher
// Graph projection for verified truth and conflict/dependency workflows.

// Constraints
CREATE CONSTRAINT ruleinstance_node_id IF NOT EXISTS
FOR (n:RuleInstance) REQUIRE n.node_id IS UNIQUE;
CREATE CONSTRAINT factinstance_node_id IF NOT EXISTS
FOR (n:FactInstance) REQUIRE n.node_id IS UNIQUE;
CREATE CONSTRAINT concept_key IF NOT EXISTS
FOR (n:Concept) REQUIRE n.key IS UNIQUE;
CREATE CONSTRAINT section_section_id IF NOT EXISTS
FOR (n:Section) REQUIRE n.section_id IS UNIQUE;
CREATE CONSTRAINT page_page_id IF NOT EXISTS
FOR (n:Page) REQUIRE n.page_id IS UNIQUE;
CREATE CONSTRAINT pageblock_block_id IF NOT EXISTS
FOR (n:PageBlock) REQUIRE n.block_id IS UNIQUE;
CREATE CONSTRAINT visa_key IF NOT EXISTS
FOR (n:Visa) REQUIRE n.key IS UNIQUE;
CREATE CONSTRAINT profile_key IF NOT EXISTS
FOR (n:ApplicantProfile) REQUIRE n.key IS UNIQUE;
CREATE CONSTRAINT topic_key IF NOT EXISTS
FOR (n:Topic) REQUIRE n.key IS UNIQUE;
CREATE CONSTRAINT office_key IF NOT EXISTS
FOR (n:Office) REQUIRE n.key IS UNIQUE;
CREATE CONSTRAINT conflictcase_id IF NOT EXISTS
FOR (n:ConflictCase) REQUIRE n.conflict_case_id IS UNIQUE;
CREATE CONSTRAINT conflictvariant_id IF NOT EXISTS
FOR (n:ConflictVariant) REQUIRE n.conflict_variant_id IS UNIQUE;
CREATE CONSTRAINT resolutiondecision_id IF NOT EXISTS
FOR (n:ResolutionDecision) REQUIRE n.resolution_decision_id IS UNIQUE;

// Verified procedural truth
MERGE (ri:RuleInstance {node_id: $rule_instance.node_id})
SET ri += $rule_instance
MERGE (c:Concept {key: $concept.key})
SET c += $concept
MERGE (s:Section {section_id: $section.section_id})
SET s += $section
MERGE (ri)-[:ABOUT]->(c)
MERGE (ri)-[:SOURCED_FROM]->(s);

FOREACH (_ IN CASE WHEN $visa IS NULL THEN [] ELSE [1] END |
  MERGE (v:Visa {key: $visa.key})
  SET v += $visa
  MERGE (ri)-[:APPLIES_TO]->(v)
);

UNWIND coalesce($profiles, []) AS profile
MERGE (p:ApplicantProfile {key: profile.key})
SET p += profile
MERGE (ri)-[:FOR_PROFILE]->(p);

// Verified facts
MERGE (fi:FactInstance {node_id: $fact_instance.node_id})
SET fi += $fact_instance
MERGE (fc:Concept {key: $fact_concept.key})
SET fc += $fact_concept
MERGE (fs:Section {section_id: $fact_section.section_id})
SET fs += $fact_section
MERGE (fi)-[:ABOUT]->(fc)
MERGE (fi)-[:SOURCED_FROM]->(fs);

// Editorial
MERGE (t:Topic {key: $topic.key})
SET t += $topic
MERGE (ts:Section {section_id: $topic_section.section_id})
SET ts += $topic_section
MERGE (ts)-[:MENTIONS_TOPIC]->(t)
FOREACH (_ IN CASE WHEN $topic.procedure_concept_key IS NULL THEN [] ELSE [1] END |
  MERGE (pc:Concept {key: $topic.procedure_concept_key})
  MERGE (t)-[:LINKS_TO_PROCEDURE]->(pc)
);

// Operational
MERGE (o:Office {key: $office.key})
SET o += $office
FOREACH (_ IN CASE WHEN $operational.entity_type = 'office_schedule' THEN [1] ELSE [] END |
  MERGE (sch:OfficeSchedule {key: $operational.key})
  SET sch += $operational
  MERGE (o)-[:HAS_SCHEDULE]->(sch)
)
FOREACH (_ IN CASE WHEN $operational.entity_type <> 'office_schedule' THEN [1] ELSE [] END |
  MERGE (n:OperationalNotice {key: $operational.key})
  SET n += $operational
  MERGE (o)-[:HAS_NOTICE]->(n)
);

// Dependency graph for selective re-extraction
MERGE (pg:Page {page_id: $page.page_id})
SET pg += $page
MERGE (pb:PageBlock {block_id: $page_block.block_id})
SET pb += $page_block
MERGE (pb)-[:PART_OF_PAGE]->(pg)
MERGE (ds:Section {section_id: $dependency.section_id})
MERGE (dri:RuleInstance {node_id: $dependency.rule_instance_id})
MERGE (ds)-[:EMITS_RULE_INSTANCE]->(dri)
MERGE (dri)-[:FEEDS_BLOCK]->(pb)
MERGE (pb)-[:DERIVED_FROM_SECTION]->(ds);

// Conflict graph
MERGE (cc:ConflictCase {conflict_case_id: $conflict_case.conflict_case_id})
SET cc += $conflict_case;
UNWIND coalesce($conflict_variants, []) AS variant
MERGE (cv:ConflictVariant {conflict_variant_id: variant.conflict_variant_id})
SET cv += variant
MERGE (cc)-[:HAS_VARIANT]->(cv)
FOREACH (_ IN CASE WHEN variant.rule_instance_id IS NULL THEN [] ELSE [1] END |
  MERGE (vri:RuleInstance {node_id: variant.rule_instance_id})
  MERGE (cv)-[:VARIANT_OF_RULE]->(vri)
);
MERGE (rd:ResolutionDecision {resolution_decision_id: $resolution.resolution_decision_id})
SET rd += $resolution
MERGE (rd)-[:RESOLVES]->(cc);

// Publish gate query: unresolved blocking conflicts for a block
MATCH (pb:PageBlock {block_id: $block_id})<-[:FEEDS_BLOCK]-(ri:RuleInstance)
MATCH (cv:ConflictVariant)-[:VARIANT_OF_RULE]->(ri)
MATCH (cc:ConflictCase)-[:HAS_VARIANT]->(cv)
WHERE cc.status = 'open' AND cc.publish_blocking = true
RETURN count(distinct cc) AS unresolved_conflicts;

// Impact query: changed section -> rules -> blocks -> pages
MATCH (s:Section {section_id: $section_id})-[:EMITS_RULE_INSTANCE]->(ri:RuleInstance)-[:FEEDS_BLOCK]->(pb:PageBlock)-[:PART_OF_PAGE]->(pg:Page)
RETURN s.section_id, collect(distinct ri.node_id) AS rule_instances, collect(distinct pb.block_id) AS blocks, collect(distinct pg.page_id) AS pages;
