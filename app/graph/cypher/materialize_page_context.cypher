// Page -> Context materialization
// Input params: $url_path, $context_key, $mapping_status

MERGE (p:Page {url_path: $url_path})
MATCH (v:VisaContext {context_key: $context_key})

MERGE (p)-[r:REPRESENTS]->(v)
SET r.mapping_status = $mapping_status
