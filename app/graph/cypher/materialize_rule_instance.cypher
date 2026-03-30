// Normative dual-layer materialization query.
// Input params: $rule_instance_id, $context_key, $concept_key, $role_type, $severity, $effective_from, $status
// ALL MERGE logic to ensure idempotency for Outbox retries.

MATCH (v:VisaContext {context_key: $context_key})
MATCH (c:Concept {concept_key: $concept_key})

// 1. Semantic Layer Edge (Dynamic relationship type handled via APOC or application-level switch if pure Cypher is needed)
// For pure Cypher without APOC dynamic type, this is often handled by specific queries per role, 
// or by setting a property on a generic semantic edge, but the spec requires specific edge types.
// Assuming the rust adapter dynamically injects the relationship type into this template:
MERGE (v)-[r:__ROLE_TYPE_PLACEHOLDER__]->(c)
SET r.severity = $severity,
    r.rule_instance_id = $rule_instance_id

// 2. Structural Layer (The Rule Instance Node)
MERGE (rule:RuleInstance {rule_instance_id: $rule_instance_id})
SET rule.status = $status,
    rule.effective_from = $effective_from

MERGE (v)-[:HAS_RULE]->(rule)
MERGE (rule)-[:CONCERNS]->(c)
