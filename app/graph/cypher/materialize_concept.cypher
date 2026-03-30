// Concept materialization
// Input params: $concept_key, $concept_type, $label_ru, $status
// Idempotent MERGE

MERGE (c:Concept {concept_key: $concept_key})
SET c.concept_type = $concept_type,
    c.label_ru = $label_ru,
    c.status = $status
