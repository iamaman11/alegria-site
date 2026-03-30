# Domain Model (V5 Canonical)

**Статус:** current domain source of truth  
**Назначение:** зафиксировать доменные инварианты, таксономию, condition DSL и graph/vector semantics.

## 1) Procedural taxonomy (замороженное ядро)

Разрешены только 8 ролей:

1. `DOCUMENT_REQUIRED`
2. `ELIGIBILITY_RULE`
3. `FEE_ITEM`
4. `TIMELINE_ITEM`
5. `WHERE_TO_APPLY`
6. `APPOINTMENT_RULE`
7. `FORM_REQUIRED`
8. `STEP`

Инвариант: 9-й роли в domain contract не существует.  
Текущее runtime-хранилище может представлять `role_type` как validated string, но допустимое множество остаётся закрытым этим документом.

## 2) Coverage gate required keys (`spoke_visa`)

### Eligibility
- `passport_validity_after_trip`
- `passport_blank_pages_min`
- `appointment_required`
- `citizenship_supported`

### Documents
- `document_required` (>=1)
- `document_format_rule`
- `photo_requirement`

### Fees and timeline
- `consular_fee`
- `processing_time`
- `submission_window`

### Apply
- `apply_channel`
- `apply_location`

Правило: missing required key => `publish_gate.status=failed`.

## 3) Condition DSL

Условия применимости rules хранятся как `condition_json` (JSONB) и вычисляются Rust policy-слоем.

- Версия: `v=1`
- Поля контекста:
  - `age_years`
  - `citizenship_code`
  - `visa_subtype`
  - `applicant_profiles`
  - `travel_purpose`
- Операторы:
  - Compare: `eq`, `neq`, `lt`, `lte`, `gt`, `gte`, `in`
  - Logical: `and`, `or`, `not`

Пример:

```json
{
  "v": 1,
  "op": "and",
  "args": [
    { "op": "gte", "field": "age_years", "value": 18 },
    { "op": "eq", "field": "citizenship_code", "value": "BY" }
  ]
}
```

Политика изменений: backward compatibility обязательна; versioning через schema registry + runtime validator.

## 4) Graph + vector contract

Фактическая графовая модель задаётся в [V5_Neo4j_Model.cypher](V5_Neo4j_Model.cypher).

Ключевые связи для аналитики:
- `ABOUT`
- `APPLIES_TO`
- `FOR_PROFILE`
- `SOURCED_FROM`
- `EMITS_RULE_INSTANCE`
- `FEEDS_BLOCK`
- `DERIVED_FROM_SECTION`
- `PART_OF_PAGE`

### GDS проекции

`visaOntology`:
- Узлы: `RuleInstance`, `FactInstance`, `Concept`, `Visa`, `ApplicantProfile`, `Section`
- Назначение: semantic neighborhood, WCC, impact analysis

`pageDependency`:
- Узлы: `Section`, `RuleInstance`, `PageBlock`, `Page`
- Назначение: selective rebuild и link planning

Примечание: 8 procedural-ролей — это атрибут `RuleInstance.role`, а не отдельные типы связей.

### Qdrant роль

Qdrant — semantic retrieval слой:
- nearest neighbors для context/fact кандидатов
- vector rerank для linking кандидатов
- QA retrieval перед verification

Qdrant не является source-of-truth; canonical truth хранится в PostgreSQL.

## 5) Enforcement matrix

Current live enforcement roots:
- live business schema: `app/db/schema.sql`
- live proto contracts: `app/contracts/proto/**`
- generated/runtime contract map: `automation/PROTO_CONTRACTS.md`
- runtime rules: [V5_Runtime_Contract.md](V5_Runtime_Contract.md)
- step rules: [STEP_CATALOG_CONTRACT.md](STEP_CATALOG_CONTRACT.md)

Reference-only domain snapshots remain in:
- `docs/V5_Postgres_DDL.sql`
- `docs/V5_Event_Contracts.json`
- `docs/V5_Schema_Registry.json`
