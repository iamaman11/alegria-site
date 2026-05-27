# V5 Truth Extraction LLM Contract

**Статус:** current runtime-aligned extraction contract
**Parent owner document:** [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)
**Назначение:** точно зафиксировать, что приложение ожидает от truth-extraction LLM, как формируется prompt, какой wire-format уходит в provider, что принимается в ответ, что отбрасывается validator-ом и что происходит после extraction.

Этот файл остаётся active satellite contract. Он не является главным owner-document для всей архитектуры.

---

## 1. Роль truth-extraction LLM

Truth-extraction LLM делает только одно:

- получает один `raw.section`
- извлекает из него procedural claim candidates
- возвращает strict JSON

Truth-extraction LLM не делает:

- не присваивает `verified`
- не пишет в `verified.rule_instances`
- не решает admissibility
- не решает contradiction/corroboration как финальный verdict
- не строит Neo4j graph
- не materialize-ит Qdrant projections
- не пишет editorial draft страницы

Его роль: **extractor / normalizer**, не authority.

Live implementation:

- extraction adapter: [app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs](/home/bose/projects/alegria-site/app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs)
- validator/adjudication logic: [app/rust/crates/primitives/src/truth_candidates.rs](/home/bose/projects/alegria-site/app/rust/crates/primitives/src/truth_candidates.rs)

---

## 2. Где truth extraction находится в pipeline

Текущий runtime порядок:

1. `serp_ingest`
2. `crawl_sources`
3. `raw_knowledge_ingestion`
4. `serp_normalize`
5. `opportunity_build`
6. `ia_build`
7. `link_recommend`
8. `draft_assemble`

Внутри `raw_knowledge_ingestion` порядок такой:

1. загрузить `raw.sections` из Postgres
2. вызвать truth-extraction LLM на каждый section
3. записать только candidate layer: `extracted.rule_candidates`
4. прогнать deterministic validator
5. прогнать adjudication
6. только потом возможно записать `verified.rule_instances`

Это происходит **до** Neo4j materialization path.

Это не обязательно до всех embeddings вообще:

- raw section embeddings/Qdrant retrieval path может существовать параллельно через `emit_raw_section_qdrant_events(...)`
- но embeddings не являются truth-verdict механизмом

---

## 3. Input contract: что передаётся в extraction

### 3.1 Runtime input object

Extractor получает `TruthExtractionInput`:

```rust
pub struct TruthExtractionInput {
    pub context_key: String,
    pub raw_section_id: i64,
    pub source_url: String,
    pub source_domain: String,
    pub heading_path: String,
    pub raw_text: String,
    pub source_snapshot_hash: String,
}
```

Смысл полей:

- `context_key` — конкретный visa/applicability context
- `raw_section_id` — id persisted `raw.sections`
- `source_url` — URL источника
- `source_domain` — домен источника
- `heading_path` — где section находился в документе
- `raw_text` — сам section text
- `source_snapshot_hash` — hash snapshot-а, к которому candidate должен быть привязан

### 3.2 System prompt

Текущий system prompt:

```text
You are an evidence-grade visa procedural extractor. Return JSON only. Do not invent facts. Extract only explicit procedural claims from the provided source section. Every claim must include exact evidence_quote plus span_start/span_end in the provided raw_text. If unsure, omit the claim.
```

Его роль:

- задаёт поведение модели
- запрещает prose
- запрещает invention
- требует evidence span
- требует omission вместо догадки

### 3.3 User prompt

Текущий user prompt:

```text
Return a JSON object with top-level key `candidates`.
Each candidate must include: role, concept_canonical_key, raw_mention, params, scope, severity, applies_to_profiles, exceptions_raw, conditions_raw, alternatives, modality_raw, derivation_type, is_numeric, is_range, is_incomplete, confidence, evidence_section_id, evidence_quote, span_start, span_end, uncertainty_flags.
Use only these roles: DOCUMENT_REQUIRED, ELIGIBILITY_RULE, FEE_ITEM, TIMELINE_ITEM, WHERE_TO_APPLY, APPOINTMENT_RULE, FORM_REQUIRED, STEP.
Use exact evidence spans from raw_text. If a field is unknown, use null or empty array/object as appropriate. Do not output prose.

context_key: {context_key}
raw_section_id: {raw_section_id}
source_url: {source_url}
source_domain: {source_domain}
heading_path: {heading_path}
raw_text:
{raw_text}
```

Важно:

- структура candidate fields задаётся именно здесь
- system prompt и user prompt работают вместе
- user prompt не просит “проверить истину”, он просит **сформировать candidate JSON**

---

## 4. Provider wire format

### 4.1 OpenAI-compatible

Endpoint:

- `https://api.openai.com/v1/chat/completions`

Request shape:

```json
{
  "model": "gpt-5.2",
  "temperature": 0.0,
  "response_format": { "type": "json_object" },
  "messages": [
    {
      "role": "system",
      "content": "..."
    },
    {
      "role": "user",
      "content": "..."
    }
  ]
}
```

Extractor читает:

- `choices[0].message.content`

### 4.2 Anthropic

Endpoint:

- `https://api.anthropic.com/v1/messages`

Request shape:

```json
{
  "model": "claude-4.5-sonnet",
  "max_tokens": 2000,
  "temperature": 0.0,
  "system": "...",
  "messages": [
    {
      "role": "user",
      "content": "..."
    }
  ]
}
```

Extractor читает:

- `content[0].text`

### 4.3 Gemini

Endpoint:

- `https://generativelanguage.googleapis.com/v1beta/models/<model>:generateContent?key=<API_KEY>`

Request shape:

```json
{
  "generationConfig": {
    "temperature": 0.0,
    "responseMimeType": "application/json"
  },
  "contents": [
    {
      "role": "user",
      "parts": [
        {
          "text": "<system_prompt>\n\n<user_prompt>"
        }
      ]
    }
  ]
}
```

Extractor читает:

- `candidates[0].content.parts[0].text`

### 4.4 Local-compatible

Endpoint:

- `SEO_TRUTH_LLM_LOCAL_ENDPOINT`
- fallback `SEO_LLM_LOCAL_ENDPOINT`

Request shape OpenAI-compatible:

```json
{
  "model": "local-truth-extraction-model",
  "temperature": 0.0,
  "response_format": { "type": "json_object" },
  "messages": [
    {
      "role": "system",
      "content": "..."
    },
    {
      "role": "user",
      "content": "..."
    }
  ]
}
```

---

## 5. Response contract: что LLM обязан вернуть

### 5.1 Canonical top-level shape

Canonical top-level key:

```json
{
  "candidates": []
}
```

Compatibility read-path всё ещё допускает:

- `claims`
- `extracted_rules`

Но canonical output — только `candidates`.

### 5.2 Candidate fields

Каждый candidate должен содержать:

- `role`
- `concept_canonical_key`
- `raw_mention`
- `params`
- `scope`
- `severity`
- `applies_to_profiles`
- `exceptions_raw`
- `conditions_raw`
- `alternatives`
- `modality_raw`
- `derivation_type`
- `is_numeric`
- `is_range`
- `is_incomplete`
- `confidence`
- `evidence_section_id`
- `evidence_quote`
- `span_start`
- `span_end`
- `uncertainty_flags`

Разрешённые `role`:

- `DOCUMENT_REQUIRED`
- `ELIGIBILITY_RULE`
- `FEE_ITEM`
- `TIMELINE_ITEM`
- `WHERE_TO_APPLY`
- `APPOINTMENT_RULE`
- `FORM_REQUIRED`
- `STEP`

### 5.3 Concrete example

Input section:

```text
Consular fee is 35 EUR. Applications are submitted at the visa centre.
```

Допустимый response:

```json
{
  "candidates": [
    {
      "role": "FEE_ITEM",
      "concept_canonical_key": "consular_fee",
      "raw_mention": "Consular fee is 35 EUR",
      "params": { "amount": 35, "currency": "EUR" },
      "scope": {},
      "severity": "mandatory",
      "applies_to_profiles": [],
      "exceptions_raw": "",
      "conditions_raw": "",
      "alternatives": [],
      "modality_raw": "",
      "derivation_type": "direct",
      "is_numeric": true,
      "is_range": false,
      "is_incomplete": false,
      "confidence": 0.94,
      "evidence_section_id": 42,
      "evidence_quote": "Consular fee is 35 EUR",
      "span_start": 0,
      "span_end": 23,
      "uncertainty_flags": []
    },
    {
      "role": "WHERE_TO_APPLY",
      "concept_canonical_key": "visa_centre_submission",
      "raw_mention": "Applications are submitted at the visa centre",
      "params": { "location_type": "visa_centre" },
      "scope": {},
      "severity": "mandatory",
      "applies_to_profiles": [],
      "exceptions_raw": "",
      "conditions_raw": "",
      "alternatives": [],
      "modality_raw": "",
      "derivation_type": "direct",
      "is_numeric": false,
      "is_range": false,
      "is_incomplete": false,
      "confidence": 0.88,
      "evidence_section_id": 42,
      "evidence_quote": "Applications are submitted at the visa centre",
      "span_start": 25,
      "span_end": 72,
      "uncertainty_flags": []
    }
  ]
}
```

---

## 6. What gets accepted, rewritten, or dropped before persistence

### 6.1 Pre-persistence normalization

`truth_extraction_llm_adapter` делает минимальный filtering/normalization:

- нормализует `role`
- принудительно выставляет `evidence_section_id = input.raw_section_id`, если модель вернула другое значение
- если `raw_mention` пустой, подставляет snippet из span
- если `evidence_quote` не совпадает с snippet, переписывает `evidence_quote = snippet`
- если `params` не object, заменяет на `{}`
- если `scope` не object, заменяет на `{}`
- если `alternatives` не array, заменяет на `[]`
- если `severity` пустой, ставит `unknown`
- если `derivation_type` пустой, ставит `direct`

### 6.2 Hard drop rules

Candidate выбрасывается до persistence, если:

- `role` неразрешённый
- пустой `concept_canonical_key`
- пустой `evidence_quote`
- `span_start >= span_end`
- `span_end > raw_text.len()`
- span не попадает на UTF-8 character boundaries
- snippet по span пустой
- `confidence` невалидный

Это всё ещё не `verified`. Это только admission в `extracted.rule_candidates`.

---

## 7. Deterministic validator: кто это и что делает

Validator — это не LLM и не человек.
Это deterministic Rust logic в [app/rust/crates/primitives/src/truth_candidates.rs](/home/bose/projects/alegria-site/app/rust/crates/primitives/src/truth_candidates.rs).

Главная функция:

- `validate_truth_candidate(candidate, raw_text)`

Она проверяет:

- допустим ли `role`
- есть ли `concept_canonical_key`
- есть ли `raw_mention`
- `params` должны быть JSON object
- `scope` должны быть JSON object
- `evidence_section_id > 0`
- есть ли `evidence_quote`
- валиден ли `span_start/span_end`
- совпадает ли `evidence_quote` с реальным substring из `raw_text`
- валиден ли `confidence`
- есть ли `source_snapshot_hash`

Дальше validator делает role-specific checks:

- `FEE_ITEM` требует amount-style params и currency
- `TIMELINE_ITEM` требует days/min_days/max_days style params
- `WHERE_TO_APPLY` требует location/office-style params
- `APPOINTMENT_RULE`, `FORM_REQUIRED`, `ELIGIBILITY_RULE` требуют непустые params
- `is_numeric` требует numeric params
- `is_range` требует range bounds
- `is_incomplete = true` ведёт в warning path
- `freshness_ambiguous`, `temporal_ambiguous`, `stale_source` ведут в warning path

Выход validator-а:

- `structured`
- `needs_hitl`
- `rejected`

`verified` на validator stage запрещён.

---

## 8. Adjudication: что происходит после validator-а

После persistence кандидаты грузятся обратно и группируются по semantic identity:

- `context_key`
- `role`
- `concept_canonical_key`

Дальше adjudicator смотрит только на `structured` candidates.

Текущая policy:

- `structured.len() >= 2` и одинаковый normalized params signature -> `verified`
- conflicting structured candidates -> `needs_hitl`
- single-source structured candidate -> `needs_hitl`
- non-structured input -> `rejected`

Source tier (`government`, `vfs`, `editorial`, etc.) используется только как signal, не как shortcut verdict.

Когда verdict = `verified`, runtime writer пишет в `verified.rule_instances`:

- `rule_candidate_id`
- `verification_method`
- `adjudication_reason`
- `publish_admissibility='admissible'`
- `freshness_class`
- `completeness_class`
- evidence fields

---

## 9. Что происходит дальше

После adjudication:

- `verified.rule_instances` становится runtime-controlled truth store
- `truth_admissibility_gate` не пускает page generation без admissible verified support
- `draft_assemble` и downstream drafting path читают только admissible verified truth

Editorial LLM — это отдельный путь. Он пишет текст секций страницы **после** truth-path, а не вместо него.

---

## 10. Relation to Qdrant, Neo4j, embeddings, and editorial LLM

### 10.1 Qdrant / embeddings

Raw-section embeddings могут появляться отдельно через raw crawl projection path:

- `emit_raw_section_qdrant_events(...)`
- VoyageAI embeddings
- semantic search over `raw_chunks_ctx` with `raw_chunks_4` fallback

Это retrieval layer, а не truth verdict.

### 10.2 Neo4j

Truth extraction не materialize-ит Neo4j напрямую.

Neo4j получает downstream materialized artifacts, не сырой output LLM.

### 10.3 Editorial draft

Editorial LLM пишет текст секций страницы только после:

- truth support bundle
- planning artifacts
- IA/page blueprint
- link recommendations
- source context chunks

То есть truth extraction не равен editorial generation.

---

## 11. Canonical current limitations

На текущем runtime важно помнить:

- truth extraction LLM делает только section-level extraction
- single-source structured candidate ещё не становится verified
- live provider readiness для extraction отдельно зависит от env:
  - `SEO_TRUTH_LLM_LOCAL_ENDPOINT|SEO_LLM_LOCAL_ENDPOINT`
  - `OPENAI_API_KEY`
  - `ANTHROPIC_API_KEY`
  - `GEMINI_API_KEY|GOOGLE_API_KEY`
- текущий recommended live path для `Step 5` и ближайшего `R3.4` — Gemini

---

## 12. Related live sources of truth

- runtime extraction adapter: [app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs](/home/bose/projects/alegria-site/app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs)
- validator/adjudication logic: [app/rust/crates/primitives/src/truth_candidates.rs](/home/bose/projects/alegria-site/app/rust/crates/primitives/src/truth_candidates.rs)
- crawl + raw ingestion runtime: [app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs](/home/bose/projects/alegria-site/app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs)
- page-build gate: [app/rust/crates/seo_application/src/seo_runtime.rs](/home/bose/projects/alegria-site/app/rust/crates/seo_application/src/seo_runtime.rs)
- runtime operations: [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md)
- extraction/adjudication roadmap status: [SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md](SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md)
