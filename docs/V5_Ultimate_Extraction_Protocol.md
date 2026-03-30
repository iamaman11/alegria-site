# V5 Ultimate Extraction Protocol
**Версия:** 1.0 — Final  
> Status note: this is the primary domain/protocol specification. Current runtime persistence, worker rollout, gating and recovery rules live in `V5_Runtime_Contract.md`, `STEP_CATALOG_CONTRACT.md`, `OPS_RUNTIME_RUNBOOK.md`, and `automation/`.

**Статус:** Операционный контракт для LLM-агентов  
**Принцип:** Layer-first → Detect Mentions → Canonicalize → Extract → Build Graph

---

## Архитектурные аксиомы

Эти правила не обсуждаются и не нарушаются ни при каких обстоятельствах.

1. **Слой определяется первым.** Layer Router завершает классификацию до любого extraction шага.
2. **Порядок пайплайна фиксирован:** Layer-first → Detect Mentions → Canonicalize → Extract → Build Graph.
3. **Приоритет — максимальная экспертность, не экономия токенов.** Шаги можно дробить для контроля качества, но при нехватке контекста промпт расширяется без ограничений.
4. **Числа только в Procedural.** Editorial и Operational не хранят числа — только ссылки на Procedural концепты.
5. **8 ролей заморожены.** Девятой роли не существует. Список: `DOCUMENT_REQUIRED`, `ELIGIBILITY_RULE`, `FEE_ITEM`, `TIMELINE_ITEM`, `WHERE_TO_APPLY`, `APPOINTMENT_RULE`, `FORM_REQUIRED`, `STEP`.
6. **Каждый факт имеет доказательство.** Тройка без `evidence_section_id` не принимается.
7. **Qdrant — поиск, не истина.** Canonical keys живут в `kb.*` PostgreSQL. Qdrant помогает найти — не решает.
8. **Symbolic matching идёт до векторного.** Exact alias → normalized alias → regex → только потом Qdrant.
9. **RuleInstance — финальная единица Procedural.** Не relation `(Visa)-[ROLE]->(Concept)`, а полноценный узел с версией, evidence и статусом.

**Иерархия слоёв.** `procedural`, `operational`, `editorial` — **truth core**: формируют verified knowledge graph, участвуют в Judge loop, HITL и graph sync. `seo` и `commercial` — **product layers**: потребляют verified knowledge, но не являются источниками истины и не проходят verification pipeline.

---

## -2. Whole Page Semantic Pass (Agent #0)

**Задача:** понять страницу как единый документ ДО sectioning.  
**Цель:** не извлекать факты, а построить глобальный контекст страницы для всех следующих шагов.  
**Выход:** `page_mode`, `dominant_layers`, `page_summary`, `global_entities`, `mixed_sections`, `cross_reference_map`.

### -2.1 Контракт ответа

```json
{
  "page_semantic_context": {
    "page_mode": "content_page|menu_page|directory_page|landing_page|utility_page",
    "dominant_layers": ["procedural", "editorial"],
    "page_summary": "Официальная страница о туристической визе в Польшу с блоками документов, сроков, сборов и FAQ.",
    "page_context_profile": {
      "country": "PL",
      "visa_type": "tourist"
    },
    "mixed_sections": [3, 7],
    "cross_reference_map": [],
    "global_entities": ["passport", "consular_fee"]
  }
}
```

### -2.2 Правило

Whole-page understanding is mandatory.  
Final factual extraction remains section/span-bound.

### -2.3 External Job Context

**Канонический термин:** `deterministic job context`

Под ним понимается внешний контекст, поставляемый orchestration/crawler layer и не генерируемый LLM. Workflow получает этот контекст как единственный источник истины (source of truth) для бизнес-параметров.

Минимальный contract:
- `country_code`
- `visa_type`
- `visa_key`
- `target_citizenship` (опционально, если страница citizenship-specific)
- `source_url`
- `source_tier`

LLM не генерирует эти поля. LLM может только подтвердить, что section согласуется с этим контекстом. Финальный authority для этих полей — `deterministic job context` + canonical page profile. Все упоминания `job context`, `crawl context` или `crawler context` в данном документе являются синонимами `deterministic job context`.

---

## -1. Page Utility Classifier

**Задача:** определить тип страницы и разрешённые режимы extraction до фильтрации DOM-блоков.

### -1.1 Контракт ответа

```json
{
  "page_mode": "content_page|menu_page|directory_page|landing_page|utility_page",
  "allow_procedural_extraction": true,
  "allow_editorial_extraction": true,
  "allow_structural_extraction": true
}
```

### -1.2 Правила

- `menu_page` / `directory_page` → procedural extraction запрещён
- `landing_page` → ограниченный extraction
- `content_page` → полный pipeline

### -1.3 Hard Override

```
If allow_procedural_extraction = false:
  → procedural pipeline MUST NOT run
  → Layer Router output for procedural MUST be ignored
  → Agent #3A MUST NOT be invoked
  → Any existing Layer Router score is irrelevant

If allow_editorial_extraction = false:
  → Agent #3C MUST NOT be invoked

If allow_structural_extraction = false:
  → Structural extraction MUST NOT run
```

**ЖЁСТКОЕ ПРАВИЛО:** `allow_*_extraction` — это hard gate, не hint. Layer Router не может override это решение. `allow_procedural_extraction = false` означает полный запрет pipeline для этой страницы, независимо от содержимого и score агентов.

---

## 0. DOM Block Relevance Filter (до sectioning)

**Задача:** отсечь navigation, menu и boilerplate до extraction.

### 0.1 block_role

- `content_main`
- `navigation`
- `footer`
- `header`
- `sidebar`
- `related_links`
- `toc`
- `breadcrumbs`
- `promo`
- `form`

### 0.2 Контракт ответа

```json
{
  "blocks": [
    {
      "dom_block_id": "b1",
      "block_role": "content_main",
      "content_relevance_score": 0.91,
      "allow_extraction": true
    },
    {
      "dom_block_id": "b2",
      "block_role": "navigation",
      "content_relevance_score": 0.05,
      "allow_extraction": false
    }
  ]
}
```

### 0.3 ЖЁСТКОЕ ПРАВИЛО

`navigation` / `menu` / `directory` blocks MUST NOT enter extraction pipeline.

---

## 1. Нарезка страницы на Sections (без LLM)

### 1.1 Правило нарезки

Section = один H-тег + весь контент до следующего H того же или высшего уровня.

Специальные блоки выделяются отдельно независимо от H-структуры:
- Таблица (даже если внутри параграфа) → отдельный section, `block_type = "table"`
- Список документов (`<ul>/<ol>` с документами) → отдельный section, `block_type = "list"`
- FAQ-блок → отдельный section на каждый вопрос-ответ, `block_type = "faq"`

Минимальный размер section: **50 символов**. Меньше → объединить с предыдущим.

Если H-тегов нет → вся страница как один section, `heading_level = 0`.

### 1.2 Что удаляется до нарезки

- Навигация, header, footer, breadcrumbs
- Cookie-баннеры, всплывающие окна
- Рекламные блоки, sidebar
- Комментарии HTML (`<!-- ... -->`)
- Скрипты и стили (`<script>`, `<style>`)

### 1.3 Контракт Section (PostgreSQL: `raw.sections`)

```json
{
  "section_id": "blake3(url + heading_text + position_on_page)",
  "source_url": "https://poland.mfa.gov.pl/en/visas/tourist",
  "url_key": "normalize_source_url(source_url)",
  "crawl_version_id": 42,
  "heading_level": 2,
  "heading_text": "Required Documents",
  "parent_heading": "Tourist Visa to Poland",
  "raw_text": "полный текст блока включая таблицы и списки",
  "content_hash": "blake3(raw_text)",
  "block_type": "requirements",
  "char_count": 847,
  "position_on_page": 3,
  "has_table": true,
  "has_list": true,
  "has_numbers": true,
  "source_domain": "poland.mfa.gov.pl",
  "source_tier": "government"
}
```

### 1.4 CAS Gate (до передачи агенту)

```
content_hash(новый) == content_hash(старый)?
  └── ДА → SKIP полностью (нет изменений)
  └── НЕТ → cheap_diff bundle:
        • numeric_diff: изменились числа / даты / диапазоны?
        • modality_diff: изменились обязательность, запрет, negation, дедлайн, minima/maxima?
        • condition_diff: изменились if/when/unless условия?
        • exception_diff: изменились исключения / альтернативы / профили?
        • entity_diff: изменились entity mentions / canonical targets?
        └── если все = false → SKIP re-extraction (вероятно только вёрстка)
        └── если хотя бы один = true → передать агенту

ЖЁСТКОЕ ПРАВИЛО: numeric_diff НИКОГДА не является единственным критерием re-extraction.
Изменение модальности, отрицания, условий, исключений или entity set считается смысловым изменением даже без изменения чисел.
```

> **Важно:** `content_hash` вычисляется детерминированно в Rust (`primitives::hash::blake3_hex`) до передачи в Python.

---

## 2. Step 1 — Layer Router (Agent #1)

**Задача:** определить `layer_scores`, `primary_layer` и `secondary_layers` до любого extraction.  
**Стартовый размер промпта (ориентир, не лимит):** ~200 токенов системной инструкции.  
**Input:** `heading_text` + `raw_text` (первые 1500 символов) + `source_tier` + `block_type` + `page_semantic_context`.

### 2.1 Системная инструкция (Layer Router)

```
Ты — классификатор информационных слоёв.

ЗАДАЧА:
Оценить КАЖДЫЙ слой независимо и вернуть score vector.

КРИТИЧЕСКИЕ ПРАВИЛА:
1. НЕ используй последовательную логику (Q1 → Q2 → ...).
2. ВСЕ 5 слоёв оцениваются независимо.
3. Каждый слой получает score ∈ [0,1].
4. primary_layer = слой с максимальным score.
5. secondary_layers = все слои с score >= 0.50 (кроме primary).

Слои:
- procedural
- operational
- editorial
- seo
- commercial

Используй не только локальный section, но и page_semantic_context,
если он нужен для точной классификации.

ЖЁСТКОЕ ПРАВИЛО: Page Utility Classifier является hard gate. Если `allow_procedural_extraction = false`, Router не может запускать procedural pipeline даже при высоком score.
Аналогично для `allow_editorial_extraction` и `allow_structural_extraction`.

Если secondary_layer confidence >= 0.70 — extraction по нему ОБЯЗАТЕЛЕН, но только если это не запрещено Utility Classifier.

Верни ТОЛЬКО JSON. Никакого текста до или после.
```

### 2.2 Контракт ответа

```json
{
  "layer_scores": {
    "procedural": 0.95,
    "operational": 0.18,
    "editorial": 0.82,
    "seo": 0.03,
    "commercial": 0.00
  },
  "primary_layer": "procedural",
  "secondary_layers": [
    { "layer": "editorial", "confidence": 0.82 }
  ],
  "reasoning_flags": {
    "blocks_publish_if_wrong": true,
    "has_ttl_date": false,
    "useful_for_article": true,
    "is_search_unit": false,
    "is_agency_offer": false
  },
  "confidence": 0.95,
  "needs_hitl": false,
  "hitl_reason": null
}
```

### 2.3 Правило secondary_layers

| Условие | Действие |
|---|---|
| secondary confidence >= 0.70 | Pipeline по secondary **обязателен** |
| secondary confidence 0.50–0.69 | Pipeline запускается, результат помечается `low_confidence` |
| secondary confidence < 0.50 | Pipeline не запускается |

> **Критично:** primary layer НЕ подавляет secondary. Раздел может содержать procedural правило, operational notice и editorial pain point одновременно. Все три pipeline запускаются.

### 2.4 Маршрутизация

| primary_layer | Pipeline | Agent |
|---|---|---|
| procedural | Pipeline A | Extractor #A |
| operational | Pipeline B | Extractor #B |
| editorial | Pipeline C | Extractor #C |
| seo | Pipeline D (детерминированный, без LLM) | — |
| commercial | Pipeline E | Extractor #E |

### 2.5 Subspan Re-routing Policy

Если section:

- содержит ≥2 слоя с score >= 0.70
ИЛИ
- разница между top-2 score < 0.12
ИЛИ
- большой и семантически смешанный

ТО:

→ разделить на subspans:
- предложения
- list items
- table rows
- FAQ items

→ повторно запустить Layer Router для каждого subspan

---

## 3. Step 2 — Entity Span Detection (Agent #2)

**Задача:** найти все значимые упоминания сущностей. Не классифицировать в детали — только тип и центральность.  
**Запускается параллельно с Layer Router**, не после.  
**Стартовый размер промпта (ориентир, не лимит):** ~300 токенов.

### 3.1 Системная инструкция

```
Твоя задача: найти все значимые упоминания объектов в тексте.
НЕ придумывай новые типы. НЕ извлекай правила — только упоминания.

Доступные типы:
concept, office, topic, pain_point, risk, task, service,
fee, timeline, location, organization, profile, legal_term, date

Для каждого упоминания:
- mention_id: стабильный ID спана внутри section (`m1`, `m2`, ...)
- raw_text: точный текст из источника (не интерпретация)
- entity_type: из списка выше
- char_start, char_end: диапазон символов в section.raw_text
- has_numeric: есть ли число, сумма или дата рядом?
- is_central: это главный объект раздела или служебное упоминание?

ЖЁСТКОЕ ПРАВИЛО: downstream join между Agent #2 и Agent #3A/#3B/#3C выполняется по mention_id / span offsets, а не по эвристическому сравнению строк.

Верни ТОЛЬКО JSON массив. Никакого текста до или после.
```

### 3.2 Контракт ответа

```json
{
  "mentions": [
    {
      "mention_id": "m1",
      "raw_text": "загранпаспорт",
      "entity_type": "concept",
      "char_start": 12,
      "char_end": 24,
      "has_numeric": false,
      "is_central": true,
      "confidence": 0.98
    },
    {
      "mention_id": "m2",
      "raw_text": "35 EUR",
      "entity_type": "fee",
      "char_start": 88,
      "char_end": 94,
      "has_numeric": true,
      "is_central": true,
      "confidence": 0.99
    },
    {
      "mention_id": "m3",
      "raw_text": "VFS Global",
      "entity_type": "organization",
      "char_start": 132,
      "char_end": 142,
      "has_numeric": false,
      "is_central": true,
      "confidence": 0.97
    },
    {
      "mention_id": "m4",
      "raw_text": "дети до 6 лет",
      "entity_type": "profile",
      "char_start": 201,
      "char_end": 215,
      "has_numeric": false,
      "is_central": false,
      "confidence": 0.93
    }
  ]
}
```

---

## 4. Step 3 — Canonical Mapping (Symbolic-first + Qdrant)

**Задача:** сопоставить каждое mention с canonical key из реестра.  
**Порядок строго:**

```
1. Exact alias match      → kb.concept_aliases (exact string match)
2. Normalized alias match → lowercase + strip + alias check
3. Regex/rule match       → registry-specific rules (напр. "VFS*" → vfs_global)
4. Qdrant similarity      → kb_canonical collection, только если 1–3 не дали результат
```

> **Почему symbolic идёт первым:** "визовый центр" и "центр подачи" — embedding может смапить неправильно. Exact alias не ошибается.

### 4.1 Пороги Qdrant

| Qdrant score | Действие |
|---|---|
| >= 0.88 | `auto_map` — маппинг без HITL |
| 0.75–0.87 | `review` — маппинг с флагом, идёт в HITL queue |
| < 0.75 | `new_candidate` — needs_hitl = true, workflow_action = `pause_for_hitl`, дальнейший triple build и Neo4j sync запрещены до resolution |

**ЖЁСТКОЕ ПРАВИЛО NULL-CANONICAL:** если `canonical_key == null`, procedural triple не строится, `MERGE (c:Concept {key: ...})` не выполняется, запись в `verified.rules` и Neo4j sync запрещены до HITL resolution.

### 4.2 Контракт ответа

```json
{
  "mappings": [
    {
      "mention_id": "m1",
      "raw_text": "загранпаспорт",
      "span": { "char_start": 12, "char_end": 24 },
      "target_registry": "kb.concepts",
      "canonical_key": "passport",
      "mapping_type": "alias",
      "match_method": "exact_alias",
      "qdrant_score": null,
      "confidence": 1.0,
      "needs_hitl": false
    },
    {
      "mention_id": "m5",
      "raw_text": "биометрический паспорт",
      "span": { "char_start": 88, "char_end": 109 },
      "target_registry": "kb.concepts",
      "canonical_key": "passport",
      "mapping_type": "alias",
      "match_method": "normalized_alias",
      "qdrant_score": null,
      "confidence": 0.97,
      "needs_hitl": false
    },
    {
      "mention_id": "m6",
      "raw_text": "справка о несудимости",
      "span": { "char_start": 201, "char_end": 221 },
      "target_registry": "kb.concepts",
      "canonical_key": null,
      "mapping_type": "new_candidate",
      "match_method": "qdrant",
      "qdrant_score": 0.61,
      "confidence": 0.61,
      "needs_hitl": true
    }
  ]
}
```

**Join Contract (Mandatory)**

```
- mapping MUST reference mention_id from EntityMentionsOutput (Agent #2)
- mapping MUST carry span { char_start, char_end } from the original mention
- mapping MUST NOT rely on raw_text string matching for downstream join
- missing mention_id → object is REJECTED; Triple Builder MUST NOT process it
```
```

---

## 5. Step 4 — Layer-Specific Extraction (Agent #3A / #3B / #3C)

После canonicalization запускается extraction по слою. Каждый extractor — отдельный агент с отдельным промптом.

Каждый extracted rule, который использует сущность из Agent #2 / Canonical Mapping, обязан вернуть либо:
- `linked_mention_ids[]`
- либо `supporting_spans[]` с `char_start` / `char_end`

Без этого Triple Builder не имеет права делать canonical join.

Если в JSON-примере procedural extractor есть rule candidate, добавить туда поле:
`"linked_mention_ids": ["m2", "m4"]`

---

### 5.1 Pipeline A — Procedural Extractor

**Стартовый размер промпта (ориентир, не лимит):** ~500 токенов.

#### Системная инструкция

```
Ты — экстрактор процедурных фактов визового процесса.
Извлекай ТОЛЬКО то, что прямо утверждается в тексте как требование
к заявителю или параметр процедуры.

ПРАВИЛА (нарушение недопустимо):
1. Никаких выводов и умозаключений. Только явно написанное.
2. Каждое число берётся дословно. Не округлять, не интерпретировать.
3. role — строго из 8 значений:
   DOCUMENT_REQUIRED, ELIGIBILITY_RULE, FEE_ITEM, TIMELINE_ITEM,
   WHERE_TO_APPLY, APPOINTMENT_RULE, FORM_REQUIRED, STEP
   Если не подходит ни одно — это не procedural, не извлекай.
4. "Может потребоваться" / "рекомендуется" → severity = "recommended".
5. Неполная информация (нет суммы, нет срока) → извлеки что есть,
   is_incomplete = true.
6. Диапазон чисел (30–45 дней, 35–80 EUR) → is_range = true.
   Публикация числа блокируется до HITL.
7. Условия ("если", "при наличии") → в conditions_raw.
8. Исключения ("кроме", "за исключением") → в exceptions_raw.
9. Альтернативы ("либо A, либо B", "вместо X — Y") →
   в alternatives: [{option_a: ..., option_b: ...}].
10. Модальность ("обязательно", "не позднее", "не ранее") →
    в modality_raw.

Верни ТОЛЬКО JSON. Никакого текста до или после.
```

#### Контракт ответа (ExtractedRuleCandidate)

```json
{
  "extracted_rules": [
    {
      "role": "FEE_ITEM",
      "concept_canonical_key": "consular_fee",
      "raw_mention": "Консульский сбор составляет 35 EUR",
      "linked_mention_ids": ["m2"],
      "params": {
        "amount": 35,
        "currency": "EUR"
      },
      "severity": "mandatory",
      "applies_to_profiles": ["adult", "student"],
      "exceptions_raw": "дети до 6 лет — бесплатно; дети 6–12 лет — 35 EUR",
      "conditions_raw": null,
      "alternatives": [],
      "modality_raw": null,
      "is_numeric": true,
      "is_range": false,
      "is_incomplete": false,
      "confidence": 0.97,
      "evidence_section_id": "{{section_id}}"
    },
    {
      "role": "ELIGIBILITY_RULE",
      "concept_canonical_key": "passport_validity",
      "raw_mention": "паспорт должен быть действителен не менее 3 месяцев после окончания поездки",
      "params": {
        "months_after_trip": 3
      },
      "severity": "mandatory",
      "applies_to_profiles": ["adult", "student", "minor"],
      "exceptions_raw": null,
      "conditions_raw": null,
      "alternatives": [],
      "modality_raw": "не менее",
      "is_numeric": true,
      "is_range": false,
      "is_incomplete": false,
      "confidence": 0.99,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

#### Разделение артефактов (важно для отладки)

Один ExtractedRuleCandidate содержит три логических уровня:

| Уровень | Поле | Назначение |
|---|---|---|
| EntityMention | `raw_mention`, `concept_canonical_key` | Что упомянуто |
| CanonicalMapping | `concept_canonical_key` + mappings из Step 3 | К чему привязано |
| RuleInstanceCandidate | `role`, `params`, `severity`, `conditions_raw` | Как это работает |

Если в extraction ошибка — сразу видно на каком уровне: сущность, маппинг или параметры.

---

### 5.2 Pipeline B — Operational Extractor

**Стартовый размер промпта (ориентир, не лимит):** ~400 токенов.

#### Типы operational entities и их params_schema

| entity_type | Обязательные params | Необязательные |
|---|---|---|
| `office_schedule` | weekdays, hours, timezone | lunch_break, by_appointment |
| `operational_notice` | notice_type, description | valid_from, valid_until, affects_visas |
| `closure_event` | date_from, date_to, reason | affects_submission, emergency_contact |
| `holiday_calendar` | country_code, year | source_url |
| `country_holiday` | date, name_local, name_en | recurring_annually |
| `submission_blackout` | date_from, date_to | reason, alternative_office |

#### Системная инструкция

```
Ты — экстрактор операционных данных внешних объектов.
Извлекай режимы работы, расписания, уведомления, праздники, закрытия.

ПРАВИЛА:
1. НЕ создавай правила подачи. Это не eligibility, не required_keys.
2. entity_type строго из: office_schedule, operational_notice,
   closure_event, holiday_calendar, country_holiday, submission_blackout
3. Каждая запись привязана к конкретному office_key или country_code.
4. Даты в формате ISO 8601 (YYYY-MM-DD).
5. Время в формате HH:MM.
6. valid_until обязателен если это временное событие.
7. TTL задаётся по lifecycle-матрице (см. Section 28):
   - office_schedule → 90d
   - operational_notice → 7d
   - closure_event / submission_blackout → до valid_until
   - holiday_calendar → yearly
   Если дата не указана, ставь `valid_until = null` и дефолт по типу выше.

Верни ТОЛЬКО JSON. Никакого текста до или после.
```

#### Контракт ответа

```json
{
  "operational_entities": [
    {
      "entity_type": "office_schedule",
      "entity_key_candidate": "pl_consulate_minsk_schedule_2026",
      "office_key_candidate": "pl_consulate_minsk",
      "country_code": "PL",
      "raw_mention": "Приём документов: пн-пт 09:00-13:00",
      "params": {
        "weekdays": "mon-fri",
        "hours_open": "09:00",
        "hours_close": "13:00",
        "timezone": "Europe/Minsk"
      },
      "valid_from": null,
      "valid_until": null,
      "ttl_days": 90,
      "confidence": 0.92,
      "evidence_section_id": "{{section_id}}"
    },
    {
      "entity_type": "closure_event",
      "entity_key_candidate": "pl_consulate_minsk_closure_20260501",
      "office_key_candidate": "pl_consulate_minsk",
      "country_code": "PL",
      "raw_mention": "Консульство закрыто 1–3 мая (праздники Польши)",
      "params": {
        "date_from": "2026-05-01",
        "date_to": "2026-05-03",
        "reason": "national_holidays",
        "affects_submission": true
      },
      "valid_from": "2026-04-01",
      "valid_until": "2026-05-04",
      "ttl_days": 7,
      "confidence": 0.95,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

---

### 5.3 Pipeline C — Editorial Topic Extractor

**Стартовый размер промпта (ориентир, не лимит):** ~400 токенов.

#### Системная инструкция

```
Ты — экстрактор редакционных тем и болей пользователей.
Твоя задача: ЧТО обсуждается, а не КАК это регулируется.

ПРАВИЛА:
1. Тема — это идея для статьи, FAQ или checklist. Формулируй её
   как полезную для читателя.
2. НЕ включай числа в topic. Только факт темы. Числа — в Procedural.
3. Боль — проблема пользователя. Формулируй от первого лица:
   "Как мне доказать доход если я ИП?"
4. Если тема связана с процедурой — укажи links_to_procedure = true
   и concept_candidate.
5. topic_type строго из: topic, travel_topic, pain_point, risk_factor,
   preparation_task, audience_need, refusal_scenario,
   destination_topic, seasonality

Верни ТОЛЬКО JSON. Никакого текста до или после.
```

#### Контракт ответа

```json
{
  "topics": [
    {
      "topic_type": "preparation_task",
      "topic_key_candidate": "prepare_bank_statement_3months",
      "human_label": "Как подготовить выписку из банка для визы",
      "raw_mention": "Для подтверждения финансов предоставьте выписку за последние 3 месяца",
      "links_to_procedure": true,
      "procedure_concept_candidate": "bank_statement",
      "country_code": "PL",
      "visa_type": "tourist",
      "audience_hint": ["adult", "self_employed"],
      "intent_type": "informational",
      "confidence": 0.89,
      "evidence_section_id": "{{section_id}}"
    },
    {
      "topic_type": "pain_point",
      "topic_key_candidate": "how_to_prove_income_self_employed",
      "human_label": "Как ИП доказать доход для шенгенской визы",
      "raw_mention": "Самозанятые и ИП предоставляют налоговую декларацию",
      "links_to_procedure": true,
      "procedure_concept_candidate": "tax_declaration",
      "country_code": "PL",
      "visa_type": "tourist",
      "audience_hint": ["self_employed"],
      "intent_type": "informational",
      "confidence": 0.91,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

---

## 6. Step 5 — Triple Builder (Code-first, Rust Activity)

**Задача:** собрать финальные тройки для Neo4j из всех результатов extraction.  
**Исполнение:** детерминированный Rust-код (не LLM).  
**Input:** canonical mappings + extracted_rules / topics / operational_entities.

### 6.1 Финальная модель Procedural в Neo4j — RuleInstance-first

Не relation-first (`Visa-[ROLE]->Concept`), а instance-first:

```cypher
// Финальная каноническая форма Procedural
(:RuleInstance {
  node_id: "stable_id(RuleInstance, pl_tourist_by, consular_fee_fee_item)",
  rule_key: "consular_fee",
  role: "FEE_ITEM",
  layer: "procedural",
  layer_version: 1,
  // Orthogonal Dimensions (обязательны при truth_status = verified)
  truth_status: "verified",
  authority_level: "official_government",
  temporal_status: "timeless",
  audience_scope: "{ visa_scope: pl_tourist_by, profile_scope: null }",
  source_provenance: "{ source_url: ..., source_domain: ..., evidence_section_id: abc123 }",
  confidence: 0.97,
  evidence_section_id: "abc123",
  source_tier: "government",
  created_at: "...",
  updated_at: "..."
})

// Связи от RuleInstance
(ri:RuleInstance)-[:ABOUT]->(c:Concept {key: "consular_fee"})
(ri:RuleInstance)-[:APPLIES_TO]->(v:Visa {key: "pl_tourist_by"})
(ri:RuleInstance)-[:FOR_PROFILE]->(ap:ApplicantProfile {key: "adult"})
(ri:RuleInstance)-[:SOURCED_FROM]->(s:Section {section_id: "abc123"})
```

> **Почему RuleInstance, не relation:**
> — Легче версионировать (каждый RuleInstance имеет свой node_id)
> — Легче снабжать evidence (прямая связь с Section)
> — Легче делать conflict workflow (два RuleInstance на одно правило — это конфликт)
> — Легче исполнять eligibility (один Cypher-запрос по APPLIES_TO)

**Orthogonal Dimension Rule**

```
Layers MUST NOT encode:
- authority     → use authority_level field
- temporal validity → use temporal_status + valid_until fields
- audience      → use audience_scope field

These properties MUST exist only in structured orthogonal dimension fields,
never as separate layers or ad-hoc node labels.
```

**ЖЁСТКОЕ ПРАВИЛО:** объект со `truth_status = verified` обязан иметь все пять dimension полей заполненными: `truth_status`, `authority_level`, `temporal_status`, `audience_scope`, `source_provenance`. Полная спецификация — Section 6.4.

### 6.2 Типы троек по всем слоям

| Слой | Triple | Пример |
|---|---|---|
| Procedural | `(RuleInstance)-[:ABOUT]->(Concept)` | `(ri_123)-[:ABOUT]->(consular_fee)` |
| Procedural | `(RuleInstance)-[:APPLIES_TO]->(Visa)` | `(ri_123)-[:APPLIES_TO]->(pl_tourist_by)` |
| Procedural | `(RuleInstance)-[:FOR_PROFILE]->(ApplicantProfile)` | `(ri_123)-[:FOR_PROFILE]->(adult)` |
| Procedural | `(RuleInstance)-[:SOURCED_FROM]->(Section)` | `(ri_123)-[:SOURCED_FROM]->(section_42)` |
| Operational | `(Office)-[:HAS_SCHEDULE]->(OfficeSchedule)` | `(pl_consulate_minsk)-[:HAS_SCHEDULE]->(schedule_2026)` |
| Operational | `(Office)-[:HAS_NOTICE]->(OperationalNotice)` | `(pl_consulate_minsk)-[:HAS_NOTICE]->(closure_0501)` |
| Operational | `(HolidayCalendar)-[:INCLUDES_HOLIDAY]->(CountryHoliday)` | `(pl_2026)-[:INCLUDES_HOLIDAY]->(pl_may_1)` |
| Editorial | `(Section)-[:MENTIONS_TOPIC]->(Topic)` | `(section_42)-[:MENTIONS_TOPIC]->(bank_statement_prep)` |
| Editorial | `(Topic)-[:LINKS_TO_PROCEDURE]->(Concept)` | `(bank_statement_prep)-[:LINKS_TO_PROCEDURE]->(bank_statement)` |
| Editorial | `(PainPoint)-[:AFFECTS_PROFILE]->(ApplicantProfile)` | `(prove_income_ip)-[:AFFECTS_PROFILE]->(self_employed)` |
| Cross-layer | `(Page)-[:PROCEDURAL_LINK]->(Visa)` | `(page_pl_tourist)-[:PROCEDURAL_LINK]->(pl_tourist_by)` |
| Cross-layer | `(Page)-[:EDITORIAL_LINK]->(Topic)` | `(page_pl_tourist)-[:EDITORIAL_LINK]->(bank_statement_prep)` |
| Cross-layer | `(Page)-[:OPERATIONAL_LINK]->(Office)` | `(page_pl_tourist)-[:OPERATIONAL_LINK]->(pl_consulate_minsk)` |

### 6.3 Контракт тройки

### 6.3.1 Join Contract между Agent #2 и Extractors

Rust Triple Builder НЕ имеет права связывать entity mappings и extracted rules по эвристическому сравнению строк (`raw_text ~= raw_mention`).

Единственный допустимый deterministic join:
- по `mention_id`, если extractor вернул ссылку на mentions
- либо по `(char_start, char_end)` span offsets внутри `section.raw_text`

Если extractor не вернул ни `mention_id`, ни span offsets для сущности, требующей canonical mapping, RuleInstance считается неполным и обязан уйти в `pause_for_hitl` / rerun extraction.

Это особенно обязательно для:
- fee / amount mentions
- timeline / deadline mentions
- profile mentions
- concept mentions, от которых зависит `concept_key`

```json
{
  "triples": [
    {
      "triple_id": "blake3(subject_key + relation_type + object_key + json_stable_serialize(params))",
      "subject_key": "ri_pl_tourist_by_consular_fee_fee_item",
      "subject_label": "RuleInstance",
      "subject_layer": "procedural",
      "relation_type": "ABOUT",
      "object_key": "consular_fee",
      "object_label": "Concept",
      "object_layer": "procedural",
      "params": {
        "amount": 35,
        "currency": "EUR",
        "severity": "mandatory"
      },
      "evidence_section_id": "{{section_id}}",
      "source_tier": "government",
      "confidence": 0.97,
      "extraction_pipeline": "procedural",
      "status": "extracted"
    }
  ]
}
```

> Статус `extracted` → идёт в verified pipeline. В Neo4j попадает только `verified`.

**Triple Identity — Invariant**

```
triple_id = hash(
  subject_key +
  relation_type +
  object_key +
  normalized_params +
  conditions_key          // blake3(normalized(conditions_raw)) or ""
)

Where:
  normalized_params = json_stable_serialize(params)        // stable sorted JSON string
  conditions_key    = blake3_hex(conditions_raw.strip())   // "" if conditions_raw is null
```

**ЖЁСТКОЕ ПРАВИЛО:** два triple с разными `params` ОБЯЗАНЫ производить разный `triple_id`. `triple_id` без `params` в хэше является нарушением контракта идемпотентности и может привести к тихой перезаписи данных.

**Decomposition Rule — сложные предложения**

Одно сложно-сочинённое предложение с ≥ 3 сущностями **декомпозируется** в несколько атомарных RuleInstances. Каждый RuleInstance — одна бинарная тройка.

Куда идут дополнительные сущности:

| Тип сущности | Куда попадает | Участвует в triple_id? |
|---|---|---|
| Канонические параметры (amount, duration, currency) | `params` | ✅ да |
| Условия применимости (channel, mode) | `params` как structured field | ✅ да |
| Профиль заявителя | `applies_to_profiles` + `FOR_PROFILE` relation | через subject_key |
| Условия в свободном тексте (if/when/unless) | `conditions_raw` → `conditions_key` в хэше | ✅ да |
| Исключения | `exceptions_raw` | ❌ не хэшируются — только для QA |
| Альтернативы | `alternatives[]` | ❌ не хэшируются |
| Citizenship / jurisdiction | `audience_scope` | через subject_key |

**ЖЁСТКОЕ ПРАВИЛО:** если условие меняет **семантический scope** правила (разный канал подачи, разное гражданство, разный возраст), оно обязано идти в `params` как структурированное поле, а **не** в `conditions_raw`. Неструктурированный `conditions_raw` допустим только для условий, которые нельзя нормализовать без LLM-суждения — и тогда он участвует в `conditions_key`.

**Пример декомпозиции:** "Граждане BY при подаче через VFS Global платят 35 EUR, но дети до 6 лет освобождены при наличии нотариального разрешения обоих родителей."

```
Triple 1: (ri_fee_main)    -[ABOUT]-> (consular_fee)
  params = { amount: 35, currency: "EUR", channel: "vfs_global" }
  applies_to_profiles = ["adult", "student"]

Triple 2: (ri_fee_exempt)  -[ABOUT]-> (consular_fee)
  params = { amount: 0, profile_age_max: 6 }
  conditions_raw = "при наличии нотариального разрешения обоих родителей"
  conditions_key = blake3("при наличии нотариального разрешения обоих родителей")

Triple 3: (ri_doc_req)     -[ABOUT]-> (notarized_parental_consent)
  conditions_raw = "требуется для освобождения от сбора для детей до 6 лет"
```

Triple 1 и Triple 2 имеют **разные triple_id** несмотря на один `object_key`, потому что `params` различаются.

### 6.4 Обязательные свойства всех узлов Neo4j

Canonical Object Envelope — полный контракт свойств для каждого knowledge-узла. Включает orthogonal dimensions из sec 46.

```json
{
  "node_id": "stable_id(kind, context, key)",
  "layer": "procedural|operational|editorial|seo|commercial",
  "layer_version": 1,
  "truth_status": "draft|extracted|verified|verified_with_limitations|deprecated|superseded|blocked",
  "authority_level": "statutory|official_government|official_partner|institutional_operational|editorial_verified|editorial_advisory|commercial_claim",
  "temporal_status": "timeless|effective_window|temporary_override|recurring|historical|superseded_by_new_rule",
  "audience_scope": {
    "visa_scope": "string | null",
    "profile_scope": "string | null",
    "jurisdiction_scope": "string | null",
    "citizenship_scope": "string | null",
    "submission_channel_scope": "string | null",
    "entry_type_scope": "string | null"
  },
  "source_provenance": {
    "source_url": "string",
    "source_domain": "string",
    "source_tier": "government|partner|institutional|editorial|commercial",
    "source_document_type": "official_page|regulation|partner_page|editorial|commercial",
    "crawl_version_id": "integer",
    "evidence_section_id": "string",
    "discovered_at": "ISO8601",
    "last_verified_at": "ISO8601"
  },
  "confidence": 0.97,
  "created_at": "ISO8601",
  "updated_at": "ISO8601"
}
```

**Backward compatibility:** поле `status` сохраняется как alias для `truth_status` на период migration. После завершения migration window `status` удаляется.

**ЖЁСТКОЕ ПРАВИЛО:** объект со `truth_status = verified` обязан иметь все пять dimension полей заполненными. Отсутствие любого из них — Schema Validation failure (sec 36.3). Для объектов со `truth_status = extracted` допустимо `audience_scope = null` и частично заполненный `source_provenance`, если evidence_section_id присутствует.

---

## 7. Step 6 — Completeness Judge (Agent #5)

**Задача:** проверить что ничего не потеряно — ни фрагменты, ни логика.  
**Input:** `raw_text` + все тройки + все unmapped mentions.  
**Стартовый размер промпта (ориентир, не лимит):** ~300 токенов системной инструкции.

### 7.1 Системная инструкция

```
Ты — судья полноты извлечения. Ты получаешь исходный текст
и все извлечённые тройки. Проверь: всё ли значимое захвачено?

Проверь 7 типов потерь:

1. ЧИСЛА: есть ли суммы, сроки, размеры, количества которые
   не вошли ни в одну тройку?

2. УСЛОВИЯ: есть ли "если", "при наличии", "при условии",
   "в случае если" которые не отражены в conditions?

3. ИСКЛЮЧЕНИЯ: есть ли "кроме", "за исключением",
   "не применяется к" которые не отражены в exceptions?

4. МОДАЛЬНОСТЬ: потеряно ли "обязательно", "не позднее",
   "не ранее", "не менее", "не более", "строго до"?
   Модальность меняет смысл правила.

5. АЛЬТЕРНАТИВЫ: потеряно ли "либо A, либо B",
   "вместо X — Y", "или предоставить"?
   Альтернативы особенно важны для ИП, несовершеннолетних,
   спонсируемых заявителей.

6. ПРОФИЛИ: упомянуты ли специфические категории заявителей
   (дети, пенсионеры, ИП, студенты) которые не привязаны
   к тройкам через FOR_PROFILE?

7. ФРАГМЕНТЫ: есть ли целые предложения без маппинга на тройку?

Для каждой потери: укажи raw_fragment, action и remediation_action.
Отдельно проверь over-extraction: есть ли в triples элементы, которых нет в исходном тексте.
Если missing_elements не пуст или hallucinated_elements не пуст,
ты ОБЯЗАН вернуть workflow_action = "reopen_extraction" или "pause_for_hitl".

Дополнительная проверка целостности:
- есть ли procedural RuleInstance без `APPLIES_TO`, если правило visa-specific?
- есть ли procedural RuleInstance без `FOR_PROFILE`, если правило profile-specific?
- есть ли RuleInstance без валидного `ABOUT -> Concept`, если роль требует concept binding?

Такие случаи считаются structural loss / graph integrity failure и не могут проходить в verified storage.

Верни ТОЛЬКО JSON. Никакого текста до или после.
```

### 7.2 Контракт ответа

```json
{
  "completeness_score": 0.88,
  "missing_elements": [
    {
      "loss_type": "exception",
      "raw_fragment": "дети до 6 лет — бесплатно",
      "reason": "возрастное исключение не отражено в params.exceptions",
      "action": "add_to_exceptions",
      "remediation_action": "targeted_patch",
      "target_triple_id": "blake3(...)"
    },
    {
      "loss_type": "alternative",
      "raw_fragment": "вместо трудовой справки ИП предоставляют налоговую декларацию",
      "reason": "альтернатива для профиля self_employed не извлечена",
      "action": "add_alternative_rule",
      "remediation_action": "rerun_extractor",
      "target_triple_id": null
    },
    {
      "loss_type": "modality",
      "raw_fragment": "не позднее чем за 90 дней до поездки",
      "reason": "временное ограничение потеряно из TIMELINE_ITEM",
      "action": "update_params",
      "remediation_action": "targeted_patch",
      "target_triple_id": "blake3(...)"
    }
  ],
  "hallucinated_elements": [],
  "workflow_action": "pause_for_hitl",
  "unmapped_raw_text": "текст который не вошёл ни в одну тройку и не объяснён",
  "needs_hitl": true,
  "hitl_reason": "альтернатива для ИП требует ручной проверки"
}
```

---

## 8. Voyage AI — что, когда и в какую коллекцию

| Момент | Что векторизуется | Коллекция | Metadata |
|---|---|---|---|
| После нарезки (до extraction) | `raw_text` section | `raw_chunks` | `section_id`, `url_key`, `source_tier`, `content_hash`, `block_type` |
| После регистрации в `kb.concepts` | `canonical_key + description + aliases (joined)` | `kb_canonical` | `canonical_key`, `concept_type`, `layer` |
| После `verified.rules` insert | `"[ROLE] [concept_key] params: ... severity: ..."` | `verified_rules` | `triple_id`, `visa_key`, `country_code`, `visa_type`, `target_citizenship`, `rule_key`, `layer` |
| После `kb.editorial_topics` insert | `human_label + raw_mention + audience_hint (joined)` | `editorial_topics` | `topic_key`, `topic_type`, `country_code`, `intent_type` |

### 8.1 Почему не вся страница, а sections

Если отправить всю страницу (3000 слов, 30 тем, 20 документов, 10 правил) — embedding станет "размазанным средним". Поиск по нему найдёт страницу, но не сможет точно локализовать факт.

**Section → Chunk → Embedding** даёт:
- Точный retrieval до конкретного paragraph
- Доказательную привязку через `evidence_section_id`
- Идемпотентность: тот же section → тот же вектор

Page-level embedding допустим **только** для coarse search и clustering — не для fact retrieval.

### 8.2 Формат payload для `verified_rules`

```json
{
  "id": "blake3(triple_id)",
  "vector": ["...voyage_embedding..."],
  "payload": {
    "triple_id": "abc123",
    "text_for_search": "FEE_ITEM consular_fee: amount=35 EUR, mandatory, Poland tourist visa, target citizenship BY",
    "visa_key": "pl_tourist_by",
    "country_code": "PL",
    "visa_type": "tourist",
    "target_citizenship": "BY",
    "role": "FEE_ITEM",
    "concept_key": "consular_fee",
    "layer": "procedural",
    "source_tier": "government",
    "confidence": 0.97
  }
}
```

**Источник business context для payload:** `visa_key`, `country_code`, `visa_type` и `target_citizenship` приходят не из LLM, а из `deterministic job context` + page_context_profile. Triple Builder обязан прокинуть эти поля детерминированно.

Если `target_citizenship` отсутствует в `deterministic job context`:
- поле не заполняется,
- citizenship-specific retrieval не считается полным,
- rule может быть сохранён только как generic visa rule без citizenship specialization.

---

## 9. Neo4j — как 5 слоёв живут в одном графе

### 9.1 Принцип единого графа

Граф **один**. Слои — это `layer` property на каждом узле, не отдельные базы и не отдельные графы.

- **Publish gates** читают: `WHERE n.layer = "procedural" AND n.status = "verified"`
- **GDS алгоритмы** работают через именованные проекции по нужным слоям
- **Cross-layer рёбра** дают GDS возможность идти через слои

### 9.2 GDS алгоритмы и проекции

```cypher
-- WCC: кластеризация тем и ключей (SEO + Editorial)
CALL gds.graph.project('seo_editorial',
  {Page: {label:'Page'}, Topic: {label:'Topic'}, Keyword: {label:'Keyword'}},
  {SEO_CLUSTER_LINK: {}, TARGETS_TOPIC: {}, IN_CLUSTER: {}}
)
CALL gds.wcc.write('seo_editorial', {writeProperty: 'cluster_id'})

-- PageRank: вес страниц для перелинковки (только SEO)
CALL gds.graph.project('seo_pages',
  {Page: {label:'Page', properties: ['pagerank']}},
  {SEO_CLUSTER_LINK: {orientation: 'NATURAL'}}
)
CALL gds.pageRank.write('seo_pages', {writeProperty: 'pagerank'})

-- NodeSimilarity: похожие темы через shared концепты (Editorial + Procedural)
CALL gds.graph.project('topic_concept',
  {Topic: {}, Concept: {}},
  {LINKS_TO_PROCEDURE: {}, RELATED_TO_CONCEPT: {}}
)
CALL gds.nodeSimilarity.stream('topic_concept') YIELD node1, node2, similarity

-- Shortest Path: от вопроса до факта (cross-layer)
MATCH (pain:PainPoint {key: 'how_to_prove_income_self_employed'})
MATCH (rule:RuleInstance {role: 'ELIGIBILITY_RULE'})
CALL gds.shortestPath.dijkstra.stream('full_graph', {
  sourceNode: pain, targetNode: rule
}) YIELD path
```

### 9.3 Cross-layer рёбра как мосты для GDS

| Ребро | Откуда → Куда | Что даёт GDS |
|---|---|---|
| `LINKS_TO_PROCEDURE` | Topic → Concept | NodeSimilarity находит темы с общими концептами |
| `TARGETS_TOPIC` | Page → Topic | WCC видит страницы и темы как один компонент |
| `PROCEDURAL_LINK` | Page → Visa | PageRank перетекает от визовых страниц к правилам |
| `MENTIONS_TOPIC` | Section → Topic | Community detection: темы упоминаемые вместе |
| `AFFECTS_PROFILE` | PainPoint → ApplicantProfile | Shortest path: вопрос → правило → факт |

### 9.4 Source Policy по слоям

| Слой | Source Policy | Что означает |
|---|---|---|
| procedural | `verified-only` | Только verified.rules / verified.facts. Extracted — не публикуется |
| operational | `verified + TTL-valid` | Verified + `valid_until > NOW()` или `ttl_days` не истёк |
| editorial | `verified-context or source-tier >= niche_agency` | Tier1 auto-verify, остальные через HITL |
| seo | `deterministic-only` | Без LLM. Только алгоритмы GDS и keyword data |
| commercial | `business-owned-only` | Только данные агентства, не из краулинга |

---

## 10. Полный поток — от HTML до Neo4j

```
[Crawlee + Playwright]
  Скачать HTML, рендерить JS
        ↓
[Нарезка на sections (без LLM)]
  По H1/H2/H3 + таблицы + списки + FAQ
  → raw.sections (PostgreSQL)
        ↓
[CAS Gate]
  CAS Decision (Delegated)
  All CAS decisions MUST follow Section 1.4 (CAS Gate).
  This flow MUST NOT implement its own simplified logic.

  Forbidden:
  - numeric-only decisions
  - partial diff checks

  Reference: → Section 1.4 is the single source of truth.
        ↓
[Voyage AI] → raw_chunks (параллельно с шагами ниже)
        ↓
[Agent #1: Layer Router] ←─────── ~200 токенов
  primary_layer + secondary_layers
        ↓
[Agent #2: Entity Span Detection] ←── ~300 токенов (параллельно с #1)
  mentions с типами
        ↓
[Step 3: Canonical Mapping]
  Exact alias → Normalized → Regex → Qdrant (kb_canonical)
        ↓
  Если canonical_key = null для критической сущности
    → pause_for_hitl / drop_incomplete_candidate
        ↓
[Agent #3A/B/C: Layer-specific Extraction] ←── ~400–500 токенов
  А: ExtractedRuleCandidate (procedural)
  B: ExtractedOperationalEntity (operational)
  C: ExtractedTopic (editorial)
        ↓
[Rust Triple Builder (code-first activity)]
  Финальные тройки (subject, relation, object, params)
        ↓
[Agent #5: Completeness Judge] ←── ~300 токенов
  Проверка 7 типов потерь + over-extraction: числа, условия, исключения,
  модальность, альтернативы, профили, фрагменты, hallucinated elements
        ↓
[Resolution Loop (Temporal)]
  missing_elements = [] и hallucinated_elements = [] ?
    → ДА: перейти дальше
    → НЕТ: workflow_action
         - targeted_patch → Rust Triple Builder patch + повторный Judge
         - rerun_extractor → повторный запуск Agent #3A/#3B/#3C + Triple Builder + Judge
         - pause_for_hitl → pause workflow, wait signal, затем повторный Judge
  До прохождения Judge запись в verified storage запрещена
        ↓
[PostgreSQL insert]
  extracted.rule_candidates / extracted.topics / kb.operational_entities
        ↓
[Verification Pipeline (Temporal + Rust activities)]
  tier=government → auto_verify
  confidence >= 0.88 → auto_verify
  иначе → HITL queue (pause workflow, wait signal)
        ↓
[verified.rules insert + registry_version bump]
        ↓
[Voyage AI] → verified_rules / editorial_topics
        ↓
[Neo4j sync (Rust activity → MERGE)]
  MERGE RuleInstance / Topic / Office / Page
  SET layer, status, confidence, updated_at
        ↓
[GDS алгоритмы]
  WCC → cluster_id
  PageRank → pagerank
  Результаты → обратно в PostgreSQL
        ↓
[Publish Gate]
  required_keys покрыты? status=verified?
  is_range=false? HTML валиден?
        ↓
[CMS Upsert (Payload API)]
  Детерминированный HTML-черновик
```

### 10.1 Failure Handling State Machine

**Состояния:**
- `extracted`: сырой результат от Agent #3A/B/C.
- `validation_failed`: не прошёл Schema Registry или Invariants.
- `mapping_blocked`: `canonical_key = null` для критической сущности.
- `paused_for_hitl`: ожидает вмешательства человека.
- `rerun_pending`: запланирован повторный запуск экстрактора.
- `patched`: применён targeted patch от Triple Builder.
- `verified`: прошёл Judge и автоматическую/ручную верификацию.
- `graph_synced`: успешно записан в Neo4j.
- `dropped`: признан невалидным или дубликатом, удалён из пайплайна.

**Разрешённые переходы:**
- `extracted` → `validation_failed` | `paused_for_hitl` | `rerun_pending`
- `validation_failed` → `rerun_pending` | `dropped`
- `paused_for_hitl` → `rerun_pending` | `dropped` | `verified`
- `rerun_pending` → `extracted`
- `patched` → `verified`
- `verified` → `graph_synced`
- `graph_synced` → `dropped` (при депрекации)

**Запрещённые переходы:**
- `validation_failed` → `verified`
- `mapping_blocked` → `verified`
- `mapping_blocked` → `graph_synced`
- `extracted` → `graph_synced` (минуя Judge/Verification)
- `paused_for_hitl` → `graph_synced` (без resolution artifact)
- `dropped` → `verified`

**ЖЁСТКОЕ ПРАВИЛО:** переход `mapping_blocked` → `graph_synced` ЗАПРЕЩЁН. Любой узел без обязательного binding обязан быть либо `dropped`, либо `paused_for_hitl`.

### 10.4 Storage Boundaries

Система строго разделяет уровни хранения данных в зависимости от их готовности.

| Storage layer | Minimum completeness | Role |
|---|---|---|
| `extracted.*` | L1 | Structurally valid raw results. |
| `verified storage` | L2/L4 | Semantically valid and verified facts. |
| Voyage AI | L3+ | Retrieval-ready rules for search. |
| Neo4j | L4 | Fully linked graph-safe RuleInstances. |
| CMS Upsert | L4 | Final drafts for publication. |

---

### 10.2 Failure Classes

Все ошибки пайплайна делятся на два класса:

**Structural failure**
- отсутствует required field
- отсутствует `params`
- отсутствует `mention_id` / span offsets для deterministic join
- отсутствует `valid_until` / `ttl_days` у временной operational entity
- отсутствует required binding key для graph-safe upsert

**Semantic failure**
- ambiguous canonical mapping
- contradiction между правилами
- потеря условий / исключений / альтернатив
- hallucinated elements
- конфликт procedural и editorial трактовки

**Правило маршрутизации:**
- structural failure → reject candidate / rerun / block storage
- semantic failure → Judge / Contradiction Gate / `pause_for_hitl`

### 10.3 Definition of Done for Procedural RuleInstance

Procedural RuleInstance считается завершённым (`done`) только если одновременно выполнены все условия:

1. Пройдена schema validation.
2. Есть `evidence_section_id`.
3. Есть корректный `params`.
4. Есть deterministic binding:
   - `ABOUT -> Concept`, если роль требует concept binding
   - `APPLIES_TO -> Visa`, если правило visa-specific
   - `FOR_PROFILE -> ApplicantProfile`, если правило profile-specific
5. Нет unresolved ambiguous canonical mapping.
6. Judge не вернул unresolved `missing_elements` или `hallucinated_elements`.
7. Rule прошёл dedup/conflict resolution.
8. Только после этого разрешены:
   - `verified` storage write
   - Neo4j sync (graph sync)
   - Voyage `verified_rules` insert

---

## 11. Токены на section — ориентиры нагрузки (не лимиты качества)

**Нумерация агентов и шагов:** Agent #0 = Whole Page Semantic Pass, Agent #1 = Layer Router, Agent #2 = Entity Span Detection, Agent #3A/#3B/#3C = layer-specific extractors, Step 5 = Triple Builder (code activity, не агент), Agent #5 = Completeness Judge. Отдельного Agent #4 нет намеренно: номер зарезервирован под future orchestration agent и не используется в текущем протоколе.

Принцип:
- Если для корректного извлечения нужен больший контекст, токены увеличиваются.
- Ограничение токенов никогда не может быть причиной потери факта/условия/исключения.
- Качество и полнота выше стоимости.

| Agent | Системная инструкция | Input (section) | Output | Итого |
|---|---|---|---|---|
| #1 Layer Router | ~200 | ~400 | ~100 | ~700 |
| #2 Entity Detection | ~300 | ~400 | ~200 | ~900 |
| Step 3 Canonical Mapping (symbolic-first, code) | 0 | deterministic inputs | deterministic output | ~0 |
| #3A Procedural Extractor | ~500 | ~400 | ~400 | ~1300 |
| Step 5 Triple Builder (Rust code-first) | 0 | deterministic inputs | deterministic output | ~0 |
| #5 Completeness Judge | ~300 | ~800 (text + triples) | ~200 | ~1300 |
| **Итого на section (базовый ориентир)** | | | | **~4200 токенов** |

Примечание: это не target на экономию. При сложных страницах допускается рост токенов до уровня, необходимого для 100% полноты.

---

## 12. Критические правила — нарушение недопустимо

1. **НИКОГДА не создавать 9-ю роль.** Если текст не вписывается в 8 ролей — это Editorial или Operational, не новый role_type.

2. **НИКОГДА не выдумывать числа.** Если число не написано явно → `is_incomplete = true`. Не округлять, не интерпретировать.

3. **НИКОГДА не хранить числа в Editorial.** Topic описывает ЧТО — не СКОЛЬКО. Числа приходят из Procedural через `LINKS_TO_PROCEDURE`.

4. **ВСЕГДА определять слой до extraction.** Без Layer Router extraction не запускается.

5. **ВСЕГДА указывать `evidence_section_id`.** Тройка без доказательства не принимается системой.

6. **ВСЕГДА `is_range = true` для диапазонов.** "30–45 дней", "35–80 EUR" — публикация блокируется до HITL.

7. **Symbolic matching до Qdrant.** Exact alias → normalized → regex → только потом вектор.

8. **RuleInstance — финальная форма Procedural.** Не relation-first, а instance-first.

9. **Secondary layer с confidence >= 0.70 — обязателен.** Не факультативен.

10. **Completeness Judge проверяет логику, не только фрагменты.** Потеря модальности и альтернативы — такая же ошибка, как потеря числа.
11. **Judge findings обязаны замыкаться в Resolution Loop.** Missing/hallucinated elements не могут просто логироваться; workflow обязан либо patch/rerun, либо pause_for_hitl.

### 12.1 Core Extraction Principle

Whole-page understanding is mandatory.  
Final facts must always be bound to section/span evidence.


---

## 13. Layer Score Vector (обязательно)

Layer Router ОБЯЗАН возвращать полный score vector по всем слоям:

```json
{
  "layer_scores": {
    "procedural": 0.93,
    "operational": 0.12,
    "editorial": 0.78,
    "seo": 0.01,
    "commercial": 0.00
  }
}
```

LLM не имеет права возвращать только primary/secondary без полного распределения.

---

## 14. Symbolic Canonical Governance

ЖЁСТКОЕ ПРАВИЛО:

- LLM НЕ выбирает `canonical_key`
- LLM только:
  - выделяет mention
  - определяет candidate_type

Canonical_key определяется ТОЛЬКО через:
- registry
- alias rules
- regex rules
- vector thresholds
- HITL

Любая попытка LLM выбрать canonical_key напрямую считается нарушением протокола.

---

## 15. Procedural Triple Construction (code-first)

Procedural triples НЕ строятся LLM.

Правило:
- LLM → ExtractedRuleCandidate
- CODE → RuleInstance + triples
- CODE также выполняет context enrichment: прокидывает page_semantic_context / page_context_profile в verified payload и graph upsert, если эти поля не вернул extractor локального section.

LLM используется только для:
- editorial
- сложные связи

---

## 16. Over-extraction Check

Completeness Judge дополнительно обязан проверять:

- есть ли данные в triples, которых нет в исходном тексте

Контракт:

```json
{
  "hallucinated_elements": []
}
```

---

## 17. Conflict Graph & Resolution Policy

Добавляются сущности:

- ConflictCase
- ConflictVariant
- ResolutionDecision

Правило:
- конфликтующие RuleInstance НЕ публикуются без ResolutionDecision

---

## 18. Dependency Graph for Re-extraction

Связи:

- Section → RuleInstance
- RuleInstance → PageBlock
- PageBlock → Page
- При изменении page_context_profile или canonical mapping page-level зависимые RuleInstance обязаны попадать в dependency graph re-extraction даже если локальный section raw_text не менялся.

Обновление section пересчитывает только зависимые сущности.

---

## 19. Registry Governance

Новый ключ допускается только если:

- нельзя выразить через alias/facet
- есть ≥2 evidence
- есть schema
- есть example
- есть consumer

---

## 20. Deprecation Policy

Статусы:
- active
- deprecated
- superseded_by
- forbidden_for_new_extraction

---

## 21. Runtime Contract (Idempotency & Replay)

Каждый шаг обязан иметь:

- input_hash
- output_hash
- idempotency_key
- retry_policy
- requires_hitl

Replay обязан фиксировать:
- prompt_version
- model_version
- registry_version
- pipeline_version

### 21.1 Ownership Matrix (Temporal vs Rust)

| Что | Кто владелец | Правило |
|---|---|---|
| Порядок шагов, retry/backoff, timeouts, wait/signal, replay | Temporal Workflow | Только orchestration, без бизнес-логики |
| HTTP/LLM/DB/Qdrant/Neo4j операции | Rust Activities | Вся бизнес-логика и side-effects |
| HITL pause/resume | Temporal Workflow + Rust HITL activity | Workflow ставится на паузу до сигнала резолюции |
| Детерминизм state transitions | Temporal Workflow | Никаких now/random/uuid в workflow-коде |

### 21.2 Pipeline Run State (обязательные поля)

В `pipeline.execution_runs` (или эквиваленте) для каждого шага фиксируются:

- `step_name`
- `status` (`pending|running|waiting_hitl|done|failed`)
- `input_hash`
- `output_hash`
- `idempotency_key`
- `requires_hitl`
- `prompt_version` (если шаг LLM)
- `model_version` (если шаг LLM)
- `registry_version`
- `pipeline_version`
- `started_at`, `finished_at`, `attempt_no`

Без этих полей шаг считается non-auditable и не проходит publish-gate.

---

## 22. Confidence & Freshness Decay

Формула:

```
effective_confidence = base_confidence * freshness_factor * source_factor
```

---

## 23. Q&A Answer Contract

Ответ всегда имеет класс:

- exact_verified_answer
- verified_but_incomplete
- needs_more_input
- no_verified_data
- editorial_context_answer

---

## 24. Contradiction Gate

Если есть конфликтующие verified rules без resolution:

→ генерация блока запрещена

---

## 25. Meta-Ontology Layer

Добавляется meta registry:

- entity_class
- relation_class
- layer_class
- evidence_class

Система становится domain-independent engine.



---

## 26. Entity Type Definitions (обязательно)

- concept — объект или артефакт, участвующий в правилах
- office — конкретная точка оказания услуги, приёма документов или административного обслуживания
- topic — тема для статьи/FAQ
- pain_point — проблема пользователя
- risk — риск отказа, задержки, блокировки или procedural failure condition
- task — действие пользователя
- service — внешний сервис или service channel, не являющийся organization node сам по себе
- fee — денежная величина
- timeline — временной параметр
- profile — категория заявителя
- organization — организация
- location — географическое место
- legal_term — юридическое понятие
- date — конкретная дата

---

## 27. Schema Registry

Каждый контракт обязан иметь:

- schema_name
- schema_version
- required_fields
- field_types
- validation_rules

### 27.1 Required Schemas (обязательно)

- LayerRouterOutput
- EntityMentionsOutput
- CanonicalMappingOutput
- ProceduralExtractionOutput
- OperationalExtractionOutput
- EditorialExtractionOutput
- TripleOutput
- CompletenessJudgeOutput
- QnAAnswerOutput

### 27.2 Rule

Every output in pipeline MUST be validated against Schema Registry.

---

## 28. Operational Lifecycle Matrix

| lifecycle_class | ttl | refresh_policy |
|-------------|-----|----------------|
| recurring_schedule | 90d | periodic |
| temporary_override | fixed | event-based |
| one_off_closure | fixed | event-based |
| blackout_window | fixed | event-based |
| country_holiday | yearly | yearly |
| office_holiday_adoption | yearly | yearly |
| notice | 7d | frequent |

---

## 29. HITL Decision Matrix

| Сценарий | Действие |
|----------|----------|
| Qdrant >= 0.88 | auto |
| Qdrant 0.75–0.87 | review |
| new_candidate | HITL |
| conflict | HITL required |
| range | block publish |
| contradiction | block publish |

### 29.1 HITL Operational Fields

Каждое HITL решение должно иметь:

- owner_role
- SLA
- allowed_actions
- escalation_path

### Пример

- new_candidate → ontology_curator
- conflict → domain_reviewer
- range → procedural_reviewer
- contradiction → block_publish + escalation



---

## 30. Procedural Scope (обязательно)

Каждый procedural rule обязан иметь scope:

```json
{
  "scope": {
    "visa_subtype": "short_stay",
    "submission_channel": "vfs_only",
    "entry_type": "single"
  }
}
```

Procedural scope обязан участвовать в:
- `rule_key` derivation (если scope влияет на смысл правила),
- contradiction detection (поиск противоречий в рамках одного scope),
- дедупликации,
- проверке применимости (applicability checks) в Q&A.

**Правило полноты:** Если `scope` не извлечён, но роль является чувствительной к области применения (scope-sensitive), правило не может получить статус выше L2.

---

## 31. Source Priority

```text
government = 1.0
official_partner = 0.9
agency = 0.7
forum = 0.4
```

Используется в effective_confidence.

---

## 32. Operational Validity Type

```json
{
  "validity_type": "fixed|recurring|unknown"
}
```

---

## 33. Derivation Type

```json
{
  "derivation_type": "direct|inferred|aggregated"
}
```

---

## 34. Extraction Mode

```json
{
  "mode": "strict|recall"
}
```

strict — только высоко уверенные
recall — максимально полное извлечение



---

## 35. Entity Type Operational Definitions

### 35.1 Core Rule

Mention types are fixed. Canonical keys evolve under governance.

### 35.2 Definitions with examples

- `concept` — объект, документ, артефакт или процедурная сущность, участвующая в verified knowledge.
  - positive: `загранпаспорт`, `bank statement`, `consular fee`
  - negative: `подача документов летом` (это topic, не concept)

- `office` — конкретная точка оказания услуги, приёма документов или административного обслуживания.
  - positive: `Consulate of Poland in Minsk`, `Visa Application Center Warsaw`
  - negative: `VFS Global` (это organization, не office)

- `topic` — тема для статьи, FAQ, guide или explainer.
  - positive: `как подготовить выписку из банка`, `праздники Польши в мае`
  - negative: `35 EUR` (это fee, не topic)

- `pain_point` — формулировка проблемы пользователя или типового затруднения.
  - positive: `как ИП доказать доход`, `что делать если нет записи`
  - negative: `паспорт должен быть действителен 3 месяца` (это procedural rule, не pain_point)

- `risk` — риск отказа, просрочки, блокировки, missing evidence или operational failure.
  - positive: `риск отказа из-за неполного пакета`, `пропуск окна подачи`
  - negative: `35 EUR` (это fee, не risk)

- `task` — действие пользователя или шаг подготовки.
  - positive: `записаться на подачу`, `подготовить фото`
  - negative: `Warsaw` (это location, не task)

- `service` — сервисная сущность или канал оказания услуги/подачи.
  - positive: `premium lounge`, `courier return`, `visa application center service`
  - negative: `VFS Global` (это organization, не service)

- `fee` — денежная величина, сбор или платежный параметр.
  - positive: `35 EUR`, `service fee`
  - negative: `3 месяца` (это timeline, не fee)

- `timeline` — срок, дедлайн, период или временной параметр.
  - positive: `до 15 дней`, `не ранее чем за 90 дней`
  - negative: `1 мая 2026` (это date, не timeline)

- `profile` — категория заявителя или аудитории.
  - positive: `student`, `self_employed`, `minor`
  - negative: `VFS Global` (это organization, не profile)

- `organization` — учреждение, компания, консульство, сервис-провайдер.
  - positive: `VFS Global`, `Consulate of Poland`
  - negative: `Minsk` (это location, не organization)

- `location` — географическое место.
  - positive: `Minsk`, `Poland`, `Warsaw`
  - negative: `туристическая виза` (это concept/topic, не location)

- `legal_term` — юридический или нормативный термин.
  - positive: `residence permit`, `temporary protection`
  - negative: `выписка из банка` (это concept, не legal_term)

- `date` — конкретная календарная дата.
  - positive: `2026-05-01`, `1 May 2026`
  - negative: `ежегодно` (это recurring validity semantics, не date)

### 35.3 Boundary Rule

Если mention нельзя стабильно отнести к одному типу без контекста, агент обязан:
- выбрать наиболее узкий технический тип,
- выставить `needs_hitl = true`,
- не создавать новый entity_type.

### 35.4 Role-to-Binding Matrix (Procedural)

| role | requires_concept | requires_visa | requires_profile | requires_params | allows_null_object |
|---|---|---|---|---|---|
| DOCUMENT_REQUIRED | yes | yes | optional | yes | no |
| ELIGIBILITY_RULE | optional | yes | yes | yes | no |
| FEE_ITEM | yes | yes | optional | yes | no |
| TIMELINE_ITEM | yes | yes | optional | yes | no |
| WHERE_TO_APPLY | yes | yes | optional | yes | no |
| APPOINTMENT_RULE | yes | yes | optional | yes | no |
| FORM_REQUIRED | yes | yes | optional | yes | no |
| STEP | optional | yes | optional | yes | no |

**ЖЁСТКОЕ ПРАВИЛО:** список procedural roles определяется только этим реестром. Любое несовпадение role между экстракторами, Schema Registry, Triple Builder и графовыми контрактами считается нарушением протокола.

### 35.5 Rule Completeness Levels

- **L0 Incomplete:** отсутствует обязательный binding (concept/visa/profile) или params.
- **L1 Structurally valid:** прошёл Schema Registry, пригоден для хранения в `extracted.*`.
- **L2 Semantically valid:** прошёл Completeness Judge (conditions/exceptions/modality сохранены).
- **L3 Retrieval-ready:** привязаны все метаданные бизнес-контекста (target_citizenship, visa_type). Пригоден для Voyage `verified_rules`.
- **L4 Publish-ready:** прошёл финальный Verification Pipeline. Пригоден для Neo4j sync и CMS upsert.

| Level | Allowed destinations |
|---|---|
| L0 | nowhere |
| L1 | `extracted.*` only |
| L2 | verified storage (non-graph-dependent facts) |
| L3 | vector retrieval layers (Voyage) |
| L4 | Neo4j + CMS + final publish |

**ЖЁСТКОЕ ПРАВИЛО:** Только L4-правила могут участвовать в Neo4j MERGE. Только L3+ могут попадать в векторный индекс для поиска.

---

## 36. Full Schema Registry

### 36.1 Registry Rule

Каждый output pipeline обязан иметь:
- `schema_name`
- `schema_version`
- `required_fields`
- `validation_rules`
- `producer_step`
- `consumer_step`

Каждая схема в реестре обязана дополнительно фиксировать:
- `failure_class` (`structural|semantic`)
- `minimum_storage_level` (L1-L4)
- `minimum_graph_level` (L4)
- `allows_hitl_override` (bool)

### 36.2 Required Schemas

#### LayerRouterOutput

```json
{
  "schema_name": "LayerRouterOutput",
  "schema_version": 1,
  "required_fields": [
    "layer_scores",
    "primary_layer",
    "secondary_layers",
    "confidence",
    "needs_hitl"
  ]
}
```

#### EntityMentionsOutput

```json
{
  "schema_name": "EntityMentionsOutput",
  "schema_version": 2,
  "required_fields": [
    "mentions[].mention_id",
    "mentions[].raw_text",
    "mentions[].entity_type",
    "mentions[].char_start",
    "mentions[].char_end",
    "mentions[].is_central",
    "mentions[].confidence"
  ]
}
```

#### CanonicalMappingOutput

```json
{
  "schema_name": "CanonicalMappingOutput",
  "schema_version": 2,
  "required_fields": [
    "mappings[].mention_id",
    "mappings[].raw_text",
    "mappings[].target_registry",
    "mappings[].canonical_key",
    "mappings[].mapping_type",
    "mappings[].match_method",
    "mappings[].confidence",
    "mappings[].needs_hitl"
  ]
}
```

`CanonicalMappingOutput` обязан быть детерминированно привязан к `EntityMentionsOutput` через `mention_id`. Mapping без `mention_id` не может участвовать в Triple Builder join.

**Optional-but-recommended fields:**
- `mappings[].char_start`
- `mappings[].char_end`
- `mappings[].qdrant_score`

#### ProceduralExtractionOutput

```json
{
  "schema_name": "ProceduralExtractionOutput",
  "schema_version": 2,
  "required_fields": [
    "extracted_rules[].role",
    "extracted_rules[].raw_mention",
    "extracted_rules[].params",
    "extracted_rules[].severity",
    "extracted_rules[].confidence",
    "extracted_rules[].evidence_section_id"
  ]
}
```

Если `params` отсутствует, procedural rule считается невалидным даже при наличии `role`, `severity` и `raw_mention`.

#### OperationalExtractionOutput

```json
{
  "schema_name": "OperationalExtractionOutput",
  "schema_version": 2,
  "required_fields": [
    "operational_entities[].entity_type",
    "operational_entities[].params",
    "operational_entities[].valid_until",
    "operational_entities[].ttl_days",
    "operational_entities[].confidence",
    "operational_entities[].evidence_section_id"
  ]
}
```

Если operational entity является временной по смыслу (closure, blackout, holiday override, outage), отсутствие `valid_until` или `ttl_days` делает entity невалидной для verified storage и graph sync.

#### EditorialExtractionOutput

```json
{
  "schema_name": "EditorialExtractionOutput",
  "schema_version": 1,
  "required_fields": [
    "topics[].topic_type",
    "topics[].human_label",
    "topics[].confidence",
    "topics[].evidence_section_id"
  ]
}
```

#### TripleOutput

```json
{
  "schema_name": "TripleOutput",
  "schema_version": 1,
  "required_fields": [
    "triples[].triple_id",
    "triples[].subject_key",
    "triples[].relation_type",
    "triples[].object_key",
    "triples[].evidence_section_id",
    "triples[].confidence"
  ]
}
```

#### CompletenessJudgeOutput

```json
{
  "schema_name": "CompletenessJudgeOutput",
  "schema_version": 2,
  "required_fields": [
    "completeness_score",
    "missing_elements",
    "hallucinated_elements",
    "workflow_action",
    "needs_hitl"
  ]
}
```

#### QnAAnswerOutput

```json
{
  "schema_name": "QnAAnswerOutput",
  "schema_version": 1,
  "required_fields": [
    "answer_class",
    "answer_text",
    "evidence_keys",
    "confidence"
  ]
}
```

### 36.3 Validation Rule

Если output не проходит schema validation:
- downstream step не запускается,
- запись в verified storage запрещена,
- событие уходит в HITL / diagnostic queue,
- Resolution Loop обязан либо исправить данные, либо зафиксировать pause_for_hitl.

---

## 37. Neo4j MERGE Contracts by Layer

### 37.1 Global MERGE Rules

- Все MERGE выполняются по `node_id` или стабильному `key`.
- `layer`, `status`, `confidence`, `updated_at` выставляются при каждом upsert.
- `created_at` выставляется только при первом создании.
- Произвольное слияние по `human_label` запрещено.

### 37.1.1 Section MERGE Contract

```cypher
MERGE (s:Section {section_id: $section_id})
ON CREATE SET s.created_at = $now
SET
  s.source_url = $source_url,
  s.heading_text = $heading_text,
  s.position_on_page = $position_on_page,
  s.raw_text = $raw_text,
  s.content_hash = $content_hash,
  s.block_type = $block_type,
  s.updated_at = $now
```

**ЖЁСТКОЕ ПРАВИЛО:** Никакой другой контракт не имеет права создавать "пустой" узел Section. Только этот детерминированный MERGE является источником истины для Section nodes.

### 37.2 Procedural MERGE Contract

```cypher
MERGE (ri:RuleInstance {node_id: $node_id})
ON CREATE SET ri.created_at = $now
SET
  ri.rule_key = $rule_key,
  ri.role = $role,
  ri.layer = 'procedural',
  ri.status = $status,
  ri.confidence = $confidence,
  ri.layer_version = $layer_version,
  ri.updated_at = $now,
  ri.evidence_section_id = $evidence_section_id

CALL {
  WITH ri
  WITH ri WHERE $concept_key IS NOT NULL
  MERGE (c:Concept {key: $concept_key})
  MERGE (ri)-[:ABOUT]->(c)
  RETURN count(*) AS _c
}

CALL {
  WITH ri
  WITH ri WHERE $visa_key IS NOT NULL
  MERGE (v:Visa {key: $visa_key})
  MERGE (ri)-[:APPLIES_TO]->(v)
  RETURN count(*) AS _v
}

CALL {
  WITH ri
  WITH ri WHERE $profile_key IS NOT NULL
  MERGE (ap:ApplicantProfile {key: $profile_key})
  MERGE (ri)-[:FOR_PROFILE]->(ap)
  RETURN count(*) AS _ap
}

MERGE (s:Section {section_id: $evidence_section_id})
  ON CREATE SET s.created_at = $now
SET
  s.source_url = $source_url,
  s.heading_text = $heading_text,
  s.position_on_page = $position_on_page,
  s.raw_text = $section_raw_text,
  s.content_hash = $section_content_hash,
  s.updated_at = $now
MERGE (ri)-[:SOURCED_FROM]->(s)

// HAS_CONDITION / HAS_EXCEPTION / ALTERNATIVE_TO создаются Rust Triple Builder по финальному triple contract

// ЖЁСТКОЕ ПРАВИЛО:
// если $concept_key IS NULL, procedural graph upsert запрещён;
// такой объект обязан уйти в pause_for_hitl до записи в Neo4j.
```

### 37.3 Operational MERGE Contract

```cypher
// relation_type вычисляется кодом, а не LLM, и ДОЛЖЕН быть типизированным.
// Запрещено сводить финальный граф operational слоя к единственному HAS_OPERATIONAL_ENTITY.
MERGE (o:Office {key: $office_key})
ON CREATE SET o.created_at = $now
SET o.layer = 'operational', o.updated_at = $now
MERGE (e:OperationalEntity {node_id: $node_id})
ON CREATE SET e.created_at = $now
SET
  e.entity_type = $entity_type,
  e.layer = 'operational',
  e.status = $status,
  e.confidence = $confidence,
  e.valid_from = $valid_from,
  e.valid_until = $valid_until,
  e.updated_at = $now
MERGE (s:Section {section_id: $evidence_section_id})
  ON CREATE SET s.created_at = $now
SET
  s.source_url = $source_url,
  s.heading_text = $heading_text,
  s.position_on_page = $position_on_page,
  s.raw_text = $section_raw_text,
  s.content_hash = $section_content_hash,
  s.updated_at = $now
MERGE (e)-[:SOURCED_FROM]->(s)
```

**Deterministic relation mapping (machine-enforced, не комментарий):**

Rust activity обязана детерминированно выбрать ровно один `relation_type` по `entity_type` и выполнить соответствующий MERGE. Выбор по строке ниже — не опциональный.

| entity_type | relation_type |
|---|---|
| `office_schedule` | `HAS_SCHEDULE` |
| `operational_notice` | `HAS_NOTICE` |
| `closure_event` | `HAS_CLOSURE` |
| `holiday_calendar` | `OBSERVES_HOLIDAY` |
| `country_holiday` | `OBSERVES_HOLIDAY` |
| `submission_blackout` | `HAS_SUBMISSION_BLACKOUT` |

**ЖЁСТКОЕ ПРАВИЛО:** entity_type вне этой таблицы не может участвовать в operational graph upsert без явного расширения реестра через HITL. Единственный допустимый generic fallback — `pause_for_hitl`.

### 37.4 Editorial MERGE Contract

```cypher
MERGE (t:Topic {key: $topic_key})
ON CREATE SET t.created_at = $now
SET
  t.human_label = $human_label,
  t.topic_type = $topic_type,
  t.layer = 'editorial',
  t.status = $status,
  t.confidence = $confidence,
  t.updated_at = $now
MERGE (s:Section {section_id: $section_id})
  ON CREATE SET s.created_at = $now
SET
  s.source_url = $source_url,
  s.heading_text = $heading_text,
  s.position_on_page = $position_on_page,
  s.raw_text = $section_raw_text,
  s.content_hash = $section_content_hash,
  s.updated_at = $now
MERGE (s)-[:MENTIONS_TOPIC]->(t)
```

### 37.5 Cross-layer Rule

Cross-layer edges создаются только если:
- source и target уже существуют,
- обе стороны имеют stable keys,
- связь подтверждена extraction contract или code rule.

### 37.6 Critical Runtime Safety Rules

1. Null canonical keys не допускаются в graph upsert.
2. Agent #2 ↔ extractor join выполняется только по `mention_id` / span offsets.
3. `allow_*_extraction` из Page Utility Classifier имеют приоритет над Layer Router.
4. Procedural rule без `params` невалиден.
5. Временная operational entity без `valid_until` и `ttl_days` невалидна.
6. `target_citizenship` и другие business-context поля приходят только из `deterministic job context`, а не из LLM.

### 37.7 Deterministic Key Derivation Contract

Для обеспечения идемпотентности и защиты от дублей, все ключи вычисляются кодом (не LLM) по следующим правилам:

- `rule_key` = blake3_hex(role + concept_key + visa_key + profile_key + json_stable_serialize(params) + evidence_section_id)
- `triple_id` = blake3_hex(subject_key + relation_type + object_key + json_stable_serialize(params) + conditions_key)
  где `conditions_key` = blake3_hex(conditions_raw.strip()) если conditions_raw не null, иначе `""`
- `node_id` = stable_id(kind, context, key) — не должен зависеть от презентационного текста.
- Изменения `raw_text`, не меняющие нормализованную семантику (параметры), ОБЯЗАНЫ сохранять старый `rule_key`.

### 37.8 Dedup & Conflict Resolution Policy

Политика дедупликации обеспечивает уникальность фактов и предотвращает "отравление" графа дублями на всех этапах пайплайна.

| Case | Detection stage | Action | Owner |
|---|---|---|---|
| identical `triple_id` | Triple Builder | collapse | Rust Triple Builder |
| same `rule_key` + same `evidence_section_id` | verified write stage | keep highest confidence, merge evidence | DB Writer |
| same semantic rule across multiple sections | graph build stage | one canonical RuleInstance + multiple `SOURCED_FROM` | Neo4j Sync Layer |
| procedural/editorial overlap | workflow merge stage | procedural dominates factual graph | Temporal Workflow |
| same binding but conflicting params | contradiction stage | block publish, `pause_for_hitl` | Judge + Contradiction Gate |
| ambiguous Qdrant delta < 0.03 | canonical mapping stage | forbid `auto_map` | Canonical Mapping Gate |

**Исполнение (Enforcement Responsibility):**
- **Collapse identical facts:** Rust Triple Builder.
- **Cross-layer dominance:** Temporal Workflow (объединение пайплайнов).
- **Multi-section merge:** DB Writer / Neo4j Sync Layer.
- **Structural validity checks:** Schema Validator.
- **Semantic contradiction detection:** Judge (Agent #5) + Temporal Workflow + Contradiction Gate.

**ЗАПРЕЩЕНО:** Существование двух `RuleInstance` с идентичными параметрами `ABOUT -> Concept` и `APPLIES_TO -> Visa` без различных `FOR_PROFILE`. Такие случаи считаются коллизией.

---

## 38. Operational Lifecycle & HITL Playbook

### 38.1 HITL Roles

- `ontology_curator` — новые canonical candidates, aliases, schema drift
- `domain_reviewer` — конфликт правил и бизнес-смысл
- `procedural_reviewer` — ranges, conditions, exceptions, publish blockers
- `operations_reviewer` — schedules, closures, notices, TTL validity

### 38.2 HITL SLA

| Scenario | Owner | SLA |
|---|---|---|
| new_candidate | ontology_curator | 2 business days |
| conflict | domain_reviewer | 1 business day |
| contradiction | domain_reviewer | same day |
| range publish block | procedural_reviewer | same day |
| operational expiry / notice ambiguity | operations_reviewer | same day |

### 38.3 Allowed Actions

| Scenario | Allowed Actions |
|---|---|
| new_candidate | approve_key, merge_into_existing, reject |
| conflict | choose_winner, supersede_old, escalate |
| contradiction | block_publish, escalate, request_reextraction |
| range | block_publish, allow_with_note, request_manual_fix |
| operational ambiguity | shorten_ttl, verify_notice, reject_entity |

#### 38.3.1 HITL Resolution Artifact

Каждое HITL-решение обязано порождать детерминированный артефакт резолюции со следующими полями:
- `decision_id`: stable hash
- `owner_role`: роль принявшего решение
- `decision_type`: `approve_key|merge|supersede|block|allow`
- `target_entity_id`: ссылка на объект
- `previous_state` / `next_state`
- `justification`: текстовое обоснование
- `affects_registry_version`: bool
- `affects_graph_sync`: bool
- `applied_at`, `applied_by`

### 38.4 Escalation Path

- `new_candidate` → ontology_curator → domain_reviewer
- `conflict` → domain_reviewer → protocol_owner
- `contradiction` → domain_reviewer → block publish immediately
- `range` → procedural_reviewer → protocol_owner if unresolved
- `operational ambiguity` → operations_reviewer → domain_reviewer

### 38.5 Publish Gate Rule

Если HITL-required item остаётся unresolved:
- procedural publish blocked,
- conflicting verified rule blocked,
- operational entity downgraded or expired,
- editorial output may survive only if it does not claim verified facts.

### 38.6 Canonical Key Lifecycle

Canonical keys в `kb.*` PostgreSQL — не статические записи. Они проходят собственный lifecycle, управляемый `ontology_curator`.

#### Deprecation Policy

Canonical key помечается `status = deprecated` когда:
- реальная сущность перестала существовать (офис закрыт, форма отменена),
- ключ был заменён более точным canonical key после ontology revision.

При депрекации:
- новые extraction outputs не могут маппироваться на deprecated key,
- все linked `RuleInstance` и `OperationalEntity` получают `status = deprecated`,
- исторические записи сохраняются для аудита с меткой `deprecated_at`.

**ЖЁСТКОЕ ПРАВИЛО:** deprecated key не удаляется — только помечается. Удаление запрещено, так как нарушает audit trail и idempotency повторных crawls.

#### Alias Merge Policy

Применяется когда два canonical keys оказались одной и той же сущностью.

- Один из ключей становится canonical (winner).
- Второй получает `merged_into = <canonical_key>`, `status = merged`.
- Все extraction outputs, ссылавшиеся на merged key, переводятся на canonical key в batch-операции с фиксацией `merge_decision_id`.
- `ontology_curator` обязан создать HITL Resolution Artifact (38.3.1) с `decision_type = merge`.

#### Alias Split Policy

Применяется когда один canonical key оказался двумя разными сущностями.

- Исходный key остаётся одним из новых ключей.
- Создаётся второй новый key.
- Все исторические extraction outputs, ссылавшиеся на исходный key, ставятся в `status = needs_review` и уходят в HITL queue для переатрибуции.
- `ontology_curator` создаёт HITL Resolution Artifact с `decision_type = split`.

#### Periodic Coverage Audit

`ontology_curator` проводит регулярный аудит реестра:

| Проверка | Частота | Критерий тревоги | Действие |
|---|---|---|---|
| Ключи без linked `verified` RuleInstance | Ежемесячно | 0 verified linkings за 90 дней | Пометить `coverage_gap`, уведомить curator |
| Ключи без свежего evidence | Ежемесячно | Нет `evidence_section_id` за 60 дней | Проверить актуальность источника |
| Orphan aliases (alias без canonical target) | Ежемесячно | Любой orphan alias | Merge или удаление alias |
| Deprecated keys без замены | Квартально | `deprecated` без `replaced_by` | Подтвердить intent или назначить замену |

**ЖЁСТКОЕ ПРАВИЛО:** ключ без ни одного `verified` RuleInstance за 90 дней не блокирует pipeline, но обязан пройти coverage review до следующего publication cycle.

---

## 39. Enforcement Matrix

### 39.1 Ownership Enforcement Matrix

| Rule | Enforced by | On failure | Forbidden Action |
|---|---|---|---|
| `canonical_key != null` before upsert | Temporal Workflow + Neo4j Sync Guard | `pause_for_hitl` | MERGE Concept / Rule |
| Rule candidate must have `params` | Schema Validator | Reject candidate | Verified Insert |
| Page Utility Classifier deny flag | Temporal Router Orchestrator | Skip pipeline | Run Extractor |
| Missing `mention_id` for join | Triple Builder | `rerun_extractor` / `pause_for_hitl` | Heuristic string join |
| Range without `is_range=true` | Judge (Agent #5) | `reopen_extraction` | Auto-verify |
| TTL/Validity missing for temporary entity | Schema Validator | Reject entity | Verified Storage |

### 39.2 Invariant Execution Matrix

| Invariant | Checked at | Blocking layer | On violation |
|---|---|---|---|
| no null canonical before procedural graph upsert | Canonical Mapping Gate + Temporal Workflow | Neo4j Sync Guard | `pause_for_hitl` |
| no procedural rule without `params` | Schema Validator | verified storage writer | reject candidate |
| no visa-specific rule without `APPLIES_TO -> Visa` | Triple Builder + Graph Integrity Check | Neo4j Sync Layer | block sync |
| no profile-specific rule without `FOR_PROFILE -> ApplicantProfile` | Triple Builder + Graph Integrity Check | Neo4j Sync Layer | block sync |
| no temporary operational entity without `valid_until` and `ttl_days` | Schema Validator + DB Writer | verified storage | reject entity |
| no citizenship-specific payload without `target_citizenship` from deterministic job context | Voyage Payload Builder | vector storage writer | block insert |
| no Agent #2 ↔ extractor join by string similarity | Triple Builder | Triple Build Stage | `rerun_extractor` / `pause_for_hitl` |

---

## 40. Invariants

#### 40.1 Pre-Storage Invariants (Verified Storage Guard)

1. **Invariant 1:** Ни один процедурный `RuleInstance` не может попасть в `verified` хранилище без `evidence_section_id`.
2. **Invariant 2:** Ни один процедурный `RuleInstance` не может попасть в `verified` хранилище без корректного `params`.
3. **Invariant 3:** Ни одна временная операционная сущность не может попасть в `verified` хранилище без полей `valid_until` и `ttl_days`.
4. **Invariant 4:** Ни один citizenship-specific vector payload не может быть сформирован без `target_citizenship` из `deterministic job context`.

#### 40.2 Pre-Graph-Sync Invariants (Neo4j Guard)

1. **Invariant 5:** Ни один процедурный `RuleInstance` не может быть синхронизирован в Neo4j без связи `ABOUT -> Concept` (кроме ролей, явно разрешающих отсутствие концепта).
2. **Invariant 6:** Ни одно правило, специфичное для визы, не может быть синхронизировано без связи `APPLIES_TO -> Visa`.
3. **Invariant 7:** Ни одно правило, специфичное для профиля, не может быть синхронизировано без связи `FOR_PROFILE -> ApplicantProfile`.
4. **Invariant 8:** Использование Agent #2 ↔ extractor join по эвристическому сходству строк ЗАПРЕЩЕНО. Любой join обязан быть детерминированным.

---

## 41. Protocol Violations

Следующие случаи классифицируются как **нарушение протокола** и ведут к немедленной блокировке пайплайна:

1.  **Registry Violation:** Использование новой `procedural role` вне замороженного списка (8 ролей).
2.  **Join Violation:** Выполнение Agent #2 ↔ extractor join по строковому сходству (в обход `mention_id`).
3.  **Graph Integrity Violation:** Попытка `graph sync` (Neo4j) с `null canonical key` или без обязательного `ABOUT` binding.
4.  **Contract Violation:** Запись процедурного правила в `verified` хранилище без поля `params`.
5.  **Scope Violation:** Сохранение визоспецифичного правила без `APPLIES_TO -> Visa`.
6.  **Profile Violation:** Сохранение профилеспецифичного правила без `FOR_PROFILE -> ApplicantProfile`.
7.  **Context Violation:** Формирование гражданство-специфичного payload в Voyage без `target_citizenship` из `deterministic job context`.
8.  **Orphan Violation:** Создание узла `Section` любым способом, кроме как через `Section MERGE Contract`.

---

## 42. Ontology Ops as Core System Capability

Ontology — не статический словарь. Это живая часть системы, и её операционная зрелость напрямую определяет качество всего extraction pipeline.

Ontology ops — это не support-функция. Это **операционное ядро надёжности** системы: без управляемой ontology extraction деградирует в неопределённые `new_candidate` + растущий ambiguity debt.

---

### 42.1 Ontology Ingestion

Formal intake pipeline для всех новых элементов ontology: canonical keys, aliases, scopes, profiles, relation conventions.

#### Statuses

| Status | Описание |
|---|---|
| `detected` | Автоматически обнаружен extraction pipeline как `new_candidate`; ещё не прошёл curator triage |
| `proposed` | Принят curator'ом в очередь на review (из `detected` или создан вручную) |
| `under_review` | Назначен reviewer; ожидает решения |
| `accepted` | Одобрен; инициирован backfill historic extractions |
| `rejected` | Отклонён; заблокирован для нового использования |
| `backfilled` | Применён к historic extractions где встречался raw mention |
| `indexed` | Внесён в canonical registry; доступен для symbolic mapping |

#### Допустимые переходы

```
detected → proposed (auto-trigger или manual curator triage)
proposed → under_review (curator assigns reviewer)
under_review → accepted | rejected
accepted → backfilled → indexed
rejected → proposed (повторно, если появились новые доказательства)
```

**ЖЁСТКОЕ ПРАВИЛО:** ни один canonical key не может участвовать в extraction pipeline до достижения статуса `indexed`. Статус `accepted` означает только одобрение — не доступность.

#### Intake Contract

Каждый новый элемент должен сопровождаться:
- `proposed_by`: `system_auto` | `curator` | `domain_reviewer`
- `evidence_count`: число extraction outputs, где встречался данный raw mention
- `entity_type`: тип из зафиксированного реестра
- `locale`: `ru` | `en` | `pl` | `*` (locale-agnostic)
- `proposed_at`, `decided_at`, `indexed_at`
- `decision_justification`: текст при `rejected` или нетривиальном `accepted`

---

### 42.2 Alias Governance

Alias — наиболее уязвимая часть ontology. Ошибка в alias распространяется на все extractions через symbolic matching (Sec 8 пайплайна).

#### Uniqueness Policy

**Alias → canonical_key mapping строго 1:1 в рамках одного `entity_type` и `locale`.**

Один и тот же alias string может принадлежать двум canonical keys только если:
- `entity_type` различается (например, `"fee"` → `service_fee` и `"concept"` → `service_fee_concept`)
- `locale` различается (например, `"ВНЖ"` в `ru` и `"VNJ"` не является алиасом в `pl`)

#### Ambiguous Alias Handling

Если один alias string отображается на несколько canonical keys одного `entity_type` и `locale`:

1. Alias помечается `ambiguous = true`
2. Автоматический `auto_map` через этот alias **блокируется**
3. Все mapping outputs с этим alias получают `needs_hitl = true`
4. `ontology_curator` обязан разрешить коллизию в рамках SLA (sec 42.5)

#### Locale-Aware Aliasing

Каждый alias имеет поле `locale`. Canonical matching проверяет `locale` соответствие:

- `locale = "*"` — универсальный alias (используется для кодов, аббревиатур, международных терминов)
- locale-specific alias приоритетнее `*` при коллизии

#### Deprecated Aliases

Alias помечается `deprecated` когда:
- стал источником систематических false positives
- был replaced более точным alias
- entity была переименована в источнике

При deprecation: все исторические mappings через deprecated alias переводятся в `review_queued` и уходят в HITL.

#### Forbidden Aliases

`forbidden_aliases` — список строк, которые никогда не могут стать alias ни для одного canonical key:

- слишком общие термины (`"документ"`, `"срок"`, `"форма"`)
- термины с неустранимой межъязыковой омонимией без context disambiguation
- термины, вызвавшие прецедентные ошибки extraction с зафиксированным `incident_id`

**ЖЁСТКОЕ ПРАВИЛО:** добавление в `forbidden_aliases` выполняется только `protocol_owner` с mandatory `justification`. Forbidden alias обнаруженный в extraction output немедленно блокирует mapping и инициирует `pause_for_hitl`.

---

### 42.3 Merge & Split Contract

Merge и Split — это не "обычные правки". Это операции с backward compatibility и graph integrity последствиями.

#### Merge: Contract Impact

| Аспект | Правило |
|---|---|
| Backward compatibility | Все extraction outputs, ссылавшиеся на merged key, автоматически считаются ссылающимися на canonical winner |
| Graph integrity | Все Neo4j узлы, связанные с merged key, перелинковываются на canonical. Merge выполняется как atomic batch |
| Vector index | Qdrant записи с merged key переиндексируются под canonical key |
| Audit | Создаётся `merge_decision_id`, связывающий все затронутые RuleInstances |
| Re-trigger | Все affected `verified` RuleInstances проходят повторный Completeness Judge pass |

#### Split: Contract Impact

| Аспект | Правило |
|---|---|
| Historic ambiguity | Все исторические extraction outputs, ссылавшиеся на исходный key, переводятся в `needs_review` |
| Re-attribution | Каждый affected RuleInstance обязан пройти ручное переназначение на один из двух новых ключей через HITL |
| Graph integrity | Neo4j узлы исходного key сохраняются до полного завершения re-attribution |
| Block on new extractions | До завершения split re-attribution новые extractions для затронутых concepts блокируются |

**ЖЁСТКОЕ ПРАВИЛО:** Split без полного плана re-attribution не начинается. `split_decision_id` должен включать список всех affected RuleInstances до старта операции.

---

### 42.4 Deprecation Lifecycle

Расширяет базовые статусы из sec 20 до полного lifecycle с явными переходами.

#### Statuses

| Status | Значение |
|---|---|
| `proposed` | Элемент в intake pipeline, ещё не active |
| `active` | Штатное состояние; доступен для extraction и mapping |
| `deprecated` | Помечен для вывода; новые extractions через него не рекомендуются, но исторически допустимы |
| `blocked_for_new_use` | Запрещён для новых extractions; grace period истёк |
| `superseded` | Заменён другим canonical key; имеет поле `replaced_by` |
| `archived` | Полностью выведен из pipeline; сохранён только для audit trail |

#### Допустимые переходы

```
proposed → active (intake completed, indexed)
active → deprecated (entity утратила нормативную актуальность)
deprecated → blocked_for_new_use (grace period ≥ 30 дней истёк)
active | deprecated → superseded (replaced_by заполнен)
blocked_for_new_use → archived (≥ 90 дней после blocking, после audit)
superseded → archived (≥ 90 дней, все mappings переведены на новый key)
```

**ЖЁСТКОЕ ПРАВИЛО:** archived key никогда не удаляется из `kb.*`. Он недоступен для extraction pipeline, но обязателен для audit. Grace period `deprecated → blocked_for_new_use` минимум 30 дней — чтобы дать время backfill historic extractions.

---

### 42.5 Curator SLA

Онтология имеет свои SLA, отдельные от HITL SLA (sec 38.2). Curator SLA определяет обязательное время реакции на каждый тип ontology-изменения.

| Тип изменения | Owner | SLA | Priority |
|---|---|---|---|
| Blocking ambiguity (extraction заблокирована) | `ontology_curator` | < 4 часов | P0 |
| Forbidden alias обнаружен в production | `ontology_curator` + `protocol_owner` | < 4 часов | P0 |
| Alias fix (deprecated или ошибочный alias) | `ontology_curator` | Same day | P1 |
| New candidate triage | `ontology_curator` | 1–2 business days | P2 |
| Deprecation decision | `ontology_curator` | 3 business days | P2 |
| Merge review | `ontology_curator` + `domain_reviewer` | Еженедельный scheduled cycle | P3 |
| Split review | `ontology_curator` + `domain_reviewer` | Scheduled (не срочно без блокера) | P3 |
| Periodic coverage audit | `ontology_curator` | Ежемесячно | P4 |

**ЖЁСТКОЕ ПРАВИЛО:** P0 SLA нарушение автоматически эскалирует на `protocol_owner` и вызывает временную остановку затронутого extraction path до разрешения.

---

### 42.6 Ambiguity Budget

Ambiguity debt накапливается незаметно и разрушает качество extraction. Ambiguity budget — это явное операционное ограничение на допустимый уровень неразрешённых онтологических проблем.

#### Tracked Metrics

| Метрика | Описание |
|---|---|
| `unresolved_candidates` | `detected` / `proposed` items, не прошедшие triage в срок SLA |
| `ambiguous_aliases` | Aliases с `ambiguous = true`, ещё не разрешёнными curator |
| `review_queued_collisions` | Semantic conflicts в verified storage, ожидающие HITL |
| `temp_fallback_mappings` | Extractions с `canonical_key = null` и временным HITL override вместо resolved key |
| `blocked_split_items` | RuleInstances в `needs_review` из-за незавершённого Split |

#### Warning Thresholds

| Метрика | Warning | Critical |
|---|---|---|
| `unresolved_candidates` | > 20 | > 50 |
| `ambiguous_aliases` | > 5 | > 15 |
| `review_queued_collisions` | > 10 | > 30 |
| `temp_fallback_mappings` | > 15 | > 40 |
| `blocked_split_items` | > 0 → immediate review | > 5 → freeze |

**ЖЁСТКОЕ ПРАВИЛО:** При достижении Critical по любой метрике — новый extraction pass блокируется до снижения всех показателей ниже Warning. Curator обязан сообщить `protocol_owner` о Critical состоянии в тот же рабочий день.

---

### 42.7 Ontology KPIs

Операционная зрелость ontology измеряется. Следующие KPIs отражаются в регулярном ontology ops отчёте (минимум ежемесячно).

| KPI | Цель | Формула |
|---|---|---|
| Median TTD (time to decision) для нового key | ≤ 2 business days | `median(accepted_at - proposed_at)` |
| Alias collision rate | < 2% | `ambiguous_aliases / total_active_aliases` |
| Auto-resolution rate | ≥ 70% | `auto_mapped_count / total_mapping_attempts` |
| % verified rules, затрагивающих deprecated key | < 5% | `rules_touching_deprecated / total_verified_rules` |
| Ontology debt backlog | < 30 items | sum(`unresolved_candidates` + `ambiguous_aliases` + `review_queued_collisions`) |
| Backfill completion rate | 100% в течение 3 дней | items в статусе `accepted` > 3 дней без перехода в `backfilled` |
| Split re-attribution completion | 100% в течение 7 дней | items в `needs_review` из split > 7 дней |

**Нарушение KPI — это не блокер pipeline, но является обязательным пунктом следующего ontology review и фиксируется в `ontology_ops_report`.**

---

### 42.8 Impact Analysis (Mandatory)

Любое изменение ontology (Class B или C) ОБЯЗАНО пройти impact analysis до начала execution.

**ЖЁСТКОЕ ПРАВИЛО:** изменение без impact analysis запрещено. Это применяется к новым canonical keys (Class B), merge/split, deprecation active keys (Class C).

#### Required Impact Fields

```
impact_analysis:
  affected_rule_instances: [list of rule_key]   // RuleInstances с этим canonical key
  affected_triples: [list of triple_id]         // triples с этим subject/object key
  affected_embeddings: [collection names]       // Qdrant коллекции для reindex
  affected_queries: [query pattern examples]    // служит для QA regression тестирования
  estimated_reindex_volume: integer             // число векторов для перезаписи
  migration_plan: string | null                 // обязателен для Class C
```

**Запрещено:** выполнять Class B/C изменение без заполненного `impact_analysis` объекта в proposal.

---

### 42.9 Ontology Versioning

Registry не имеет только одного "текущего состояния" — он имеет историю версий.

**Принцип:** каждое успешно applied registry change создаёт новый `registry_version_id`. Graph и embeddings обязаны ссылаться на версию, при которой они были созданы.

#### Version Contract

```
registry_version:
  version_id: "ontology_v{N}"         // монотонно возрастающий
  applied_at: timestamp
  change_class: A | B | C
  proposal_id: string
  delta: { added: [], deprecated: [], merged: [], split: [] }
  snapshot_ref: string                 // pointer на полный snapshot registry в этой версии
```

#### Rollback Requirement

- Откат Class A: автоматический, без migration
- Откат Class B: ручной через curator, с re-triage affected candidates
- Откат Class C: обязателен `rollback_plan` в proposal; rollback выполняется через reverse migration

**ЖЁСТКОЕ ПРАВИЛО:** `registry_version_id` обязан присутствовать в `source_provenance` каждого verified RuleInstance. Это позволяет воспроизводить состояние knowledge graph на любой момент времени.

---

### 42.10 Conflict Model

Не все конфликты в ontology одинаковы. Явная классификация позволяет применять правильную resolution strategy.

#### Conflict Types

| Тип | Описание | Resolution Strategy |
|---|---|---|
| `duplicate` | Два alias → тот же canonical key, тот же entity_type, тот же locale | Auto-collapse: один удаляется, остаётся canonical |
| `partial_overlap` | Два canonical key имеют пересекающийся семантический диапазон, но не идентичны | Curator decision: merge, split, or scope-partition |
| `ambiguity` | Один alias → два разных canonical key, одинаковые entity_type и locale | Block auto-map; curator assigns dominant mapping + creates disambiguation note |
| `cross_type_collision` | Один alias string → разные canonical keys в разных entity_type | Allowed: context-based disambiguation via entity_type lookup |

#### Conflict Lifecycle

```
conflict_detected
    ↓
conflict_classified (auto, by conflict_type)
    ↓
curator_assigned (per SLA from 42.5)
    ↓
resolution_proposed
    ↓
approved → registry_update + audit_log
rejected → re-classify or escalate
```

**ЖЁСТКОЕ ПРАВИЛО:** `ambiguity` conflict блокирует auto-mapping через этот alias до resolution. `partial_overlap` не блокирует extraction, но помечает affected mappings `review_recommended = true`.

---

### 42.11 Retrieval Sync Rule

Ontology change → retrieval не синхронизируются автоматически без явного trigger.

**Принцип:** каждое registry change обязано явно указывать, какие retrieval коллекции затронуты. Несинхронизированные embeddings = stale search results = retrieval drift.

#### Sync Contract

```
ontology_change → MUST trigger:
  1. embedding_invalidation: mark affected vectors as stale
  2. reindex_queue: add affected canonical keys to reindex batch
  3. vector_refresh: execute reindex before next extraction pass using affected keys
```

#### Sync by Change Class

| Change Class | Sync Required | Timing |
|---|---|---|
| Class A (alias add, typo fix) | Only if new alias creates new searchable surface | Async, next batch |
| Class B (new canonical key, new subtype) | Yes: `kb_canonical` collection update | Within 24h |
| Class C (merge, split, deprecate) | Yes: full reindex of affected vectors | Before next extraction using affected keys; Class C change blocks pipeline until sync done |

**ЖЁСТКОЕ ПРАВИЛО:** Class C ontology change блокирует extraction pipeline для affected keys до завершения полного vector refresh. Запуск extraction с устаревшими embeddings после Class C — это нарушение протокола.

---

## 43. Three Planes Architecture

Система разделена на три концептуальные плоскости. Это не разные базы данных и не уровни хранения — это три принципиально разных вида сущностей с разными требованиями к достоверности, изменяемости и ownership.

Смешение плоскостей — это архитектурная ошибка, не просто плохая практика. Правила этого раздела предотвращают ситуации когда:
- retrieval results повышают truth status
- serving artifacts становятся источником canonical facts
- LLM напрямую мутирует verified knowledge

**Ключевой принцип:** данные текут только вниз — `Truth → Retrieval → Serving`. Обратный поток запрещён.

---

### 43.1 Truth Plane

**Определение:** единственный авторитетный источник verified knowledge в системе.

#### Объекты

| Объект | Хранилище | Минимальный уровень |
|---|---|---|
| Canonical keys, aliases, scopes | `kb.canonical`, `kb.aliases` | indexed (sec 42.1) |
| Verified RuleInstances | `verified.rules` | L4 (sec 35.5) |
| Verified OperationalEntities | `kb.operational_entities` | L4 |
| Verified EditorialTopics | `verified.topics` | L4 |
| Evidence bindings | `raw.sections` (source) | L1+ |
| HITL Resolution Artifacts | `kb.hitl_decisions` | — |
| Neo4j graph nodes | Neo4j | L4 |

#### Обязательные свойства всех объектов Truth Plane

- **versioned**: `registry_version`, `layer_version` на каждом узле
- **idempotent**: stable keys через blake3 derivation (sec 37.7); повторный run даёт тот же key
- **auditable**: `created_at`, `updated_at`, `decision_id` при любом изменении
- **provenance-bound**: `evidence_section_id` обязателен (исключения явно документированы)
- **status-gated**: попадание в Truth Plane только через полный verification chain (L2+ для verified storage, L4 для Neo4j)

#### Кто пишет в Truth Plane

**Только:** Temporal Workflow + Rust activities, прошедшие полный verification chain.

LLM не является прямым writer'ом Truth Plane. LLM производит extraction candidates — они становятся Truth только после Schema Validator → Judge → Verification Pipeline → HITL (если required).

---

### 43.2 Retrieval Plane

**Определение:** механизм нахождения релевантного контекста. Не источник истины — инструмент поиска.

#### Объекты

| Объект | Хранилище | Назначение |
|---|---|---|
| raw_chunks embeddings | Voyage AI | Coarse search, clustering |
| verified_rules vectors | Voyage AI | Fact retrieval по контексту запроса |
| editorial_topics vectors | Voyage AI | Topical linking, related content |
| kb_canonical collection | Qdrant | Alias/concept matching (symbolic fallback) |
| Section embeddings | Voyage AI | Coarse page-level search |

#### Свойства

- **derived**: Retrieval Plane строится из Truth Plane. Не наоборот.
- **rebuildable**: весь Retrieval Plane может быть удалён и восстановлен из Truth Plane без потери verified knowledge.
- **search-optimized**: объекты оптимизированы для ANN similarity, а не для нормативной точности.
- **potentially stale**: Retrieval artifacts могут отставать от Truth Plane. Это допустимо — staleness в retrieval не означает ошибку в verified knowledge.

#### Кто пишет в Retrieval Plane

Только детерминированные indexing activities (Rust/Python, не LLM). Запись происходит после перехода факта в L3+ (Voyage) или при canonical key acceptance (Qdrant).

**Qdrant match score — это не confidence в truth**, а confidence в similarity. Аксиома 7 документа: "Qdrant — поиск, не истина".

---

### 43.3 Serving Plane

**Определение:** то, что отдаётся пользователю или CMS. Собирается из Truth + Retrieval, не является источником authority.

#### Объекты

| Объект | Описание |
|---|---|
| Q&A responses | Assembled answers на основе retrieved facts |
| CMS blocks / fragments | Детерминированные HTML-черновики из verified rules |
| Checklists | Structured UI из verified procedural rules |
| Explainers | Editorial synthesis + procedural links |
| Structured UI outputs | Любые user-facing представления |

#### Свойства

- **ephemeral**: Serving artifacts не являются source of truth. Они могут быть пересобраны в любой момент из Truth + Retrieval без потери данных.
- **assembled**: Serving Plane не производит факты — он собирает и представляет их.
- **no canonical authority**: ни один объект Serving Plane не может создавать canonical keys, verified facts или менять статусы в Truth Plane.
- **not indexed**: Serving outputs не попадают в Retrieval Plane как indexable artifacts.

#### Кто пишет в Serving Plane

Assembly services, CMS API. LLM участвует в generation, но под supervision verified facts из Truth Plane.

---

### 43.4 Plane Mapping Table

| Plane | Core objects | Primary storage | Allowed writes | Запрещено |
|---|---|---|---|---|
| **Truth** | Canonical keys, verified facts, graph nodes, evidence | PostgreSQL `kb.*`, `verified.*`, Neo4j (L4) | Verified workflow (Temporal + Rust) | LLM direct mutation; retrieval-driven promotion; serving-driven fact creation |
| **Retrieval** | Vectors, ANN indexes, topic links | Voyage AI, Qdrant | Deterministic indexing от Truth (L3+) | Truth decisions; canonical key creation; status mutation |
| **Serving** | Q&A, CMS blocks, checklists, explainers | Ephemeral / CMS | Assembly из Truth + Retrieval | Canonical authority; verified fact creation; registry writes |

---

### 43.5 Plane Boundary Rules

Эти правила не нарушаются ни при каких обстоятельствах.

1. **Retrieval не повышает truth status.** Qdrant score ≥ 0.88 → кандидат на `auto_map`, а не на `verified`. Финальное решение принимает verification chain или HITL.

2. **LLM не пишет напрямую в Truth Plane.** LLM produces extraction candidates. Truth writes — только через Temporal Workflow + Rust activities + verification gate.

3. **Truth Plane независим от Retrieval Plane.** Qdrant и Voyage AI можно полностью пересобрать из Truth Plane. Их недоступность или очистка не затрагивает verified knowledge.

4. **Serving Plane не создаёт canonical факты.** Никакой Q&A ответ, CMS блок или checklist не может стать источником canonical key, verified rule или graph node.

5. **Удаление Retrieval artifact не меняет verified truth.** Wiping Voyage или Qdrant — операция на Retrieval Plane, не на Truth Plane.

6. **Serving outputs не индексируются как retrieval artifacts.** Пересобранный Q&A ответ не попадает обратно в Qdrant или Voyage как новый chunk.

7. **Данные текут только вниз.** `Raw Source → Truth → Retrieval → Serving`. Обратный поток в любом направлении — нарушение протокола.

---

### 43.6 Allowed Transformations

Полный список разрешённых переходов между плоскостями.

| From | To | Операция | Исполнитель | Constraint |
|---|---|---|---|---|
| Raw Source | Truth | Extraction pipeline (sec 10) | Temporal + Agents + Rust | Только через полный verification chain |
| Truth | Truth | Verification, MERGE, HITL resolution | Temporal + Rust + HITL | Requires status gate (L2+→L4) |
| Truth | Retrieval | Indexing, vectorization | Rust activity (deterministic) | Только L3+ facts |
| Truth | Serving | Fact assembly | Assembly service | Только L4 facts |
| Retrieval | Serving | Context retrieval для Q&A | Q&A / CMS generation | Результат — suggestion, не fact |
| Retrieval | Truth | **ЗАПРЕЩЕНО** | — | Retrieval match не становится verified |
| Serving | Truth | **ЗАПРЕЩЕНО** | — | Generated output не становится canonical |
| Serving | Retrieval | **ЗАПРЕЩЕНО** | — | User-facing output не индексируется |

**Нарушение любого ЗАПРЕЩЕНО правила классифицируется как Protocol Violation (sec 41) и ведёт к немедленной блокировке затронутого pipeline path.**

---

## 44. Frozen Roles and Controlled Semantic Extension

**Принцип:** `roles frozen, semantics extensible`

8 procedural roles (аксиома 5) определяют **структурный тип** факта — его graph shape, binding contract и serving semantics. Они не описывают предметную область исчерпывающе. Предметное богатство живёт не в новых ролях, а в контролируемых semantic extension механизмах.

Отсутствие явного extension mechanism приводит к тому, что команда начинает изобретать скрытые подроли, насиловать `params`, или всё же пытаться ввести 9-ю роль. Этот раздел делает extension channel официальным, а не дырой.

---

### 44.1 Semantic Specialization Contract

Semantic richness procedural rule выражается исключительно через следующие поля. Ни одно из них не создаёт новую роль:

| Поле | Назначение |
|---|---|
| `params` | Количественные и качественные свойства факта |
| `conditions_raw` | Условия применимости (if/when/unless) |
| `exceptions_raw` | Исключения из правила |
| `alternatives` | Альтернативные пути (либо A, либо B) |
| `modality_raw` | Модальность (обязательно, не позднее, не ранее) |
| `applies_to_profiles` | Ограничение по категории заявителя |
| `subtype` | Семантическое семейство внутри роли (см. 44.2) |
| `facets` | Структурированные булевые/enum свойства (см. 44.3) |

**ЖЁСТКОЕ ПРАВИЛО:** Если различие между двумя фактами выражается через любое из этих полей, новая роль не создаётся. Role меняется только если меняется структурный тип.

---

### 44.2 Subtype Model

`subtype` — семантическое семейство внутри роли. Не меняет binding contract родительской роли. Управляется `ontology_curator` через intake pipeline (sec 42.1).

#### Свойства subtype

- Строго привязан к одной роли: subtype `financial_proof` принадлежит `DOCUMENT_REQUIRED`, не может быть использован для `FEE_ITEM`
- Не создаёт новые required_fields в Schema Registry
- Не меняет graph shape (тот же Neo4j узел, те же обязательные отношения)
- Версионируется как часть ontology (registry_version)
- Может эволюционировать: новые subtypes добавляются через ontology intake, не через protocol revision

#### Canonical Subtype Registry (начальный набор)

| role | subtypes |
|---|---|
| `DOCUMENT_REQUIRED` | `financial_proof`, `identity_document`, `travel_document`, `sponsor_letter`, `medical_document`, `legal_authorization` |
| `ELIGIBILITY_RULE` | `age_requirement`, `nationality_restriction`, `status_requirement`, `financial_requirement`, `health_requirement` |
| `FEE_ITEM` | `consular_fee`, `service_fee`, `biometric_fee`, `courier_fee` |
| `TIMELINE_ITEM` | `processing_time`, `validity_period`, `advance_booking_window`, `submission_window`, `grace_period` |
| `WHERE_TO_APPLY` | `consulate_submission`, `vac_submission`, `postal_submission`, `online_submission` |
| `APPOINTMENT_RULE` | `biometric_appointment`, `interview_appointment`, `document_submission_appointment` |
| `FORM_REQUIRED` | `application_form`, `cover_letter`, `declaration_form`, `sponsor_form` |
| `STEP` | `preparation_step`, `submission_step`, `post_submission_step`, `entry_step` |

Subtype `null` допустим — не каждый факт нуждается в специализации.

---

### 44.3 Facet Model

`facets` — объект с ключами из фиксированного `facet_keys` реестра. Управляется `ontology_curator`. Значения: `boolean`, `string enum`, или `null`.

#### Формат

```json
{
  "role": "DOCUMENT_REQUIRED",
  "subtype": "financial_proof",
  "facets": {
    "document_family": "bank_evidence",
    "accepts_alternatives": true,
    "profile_sensitive": true,
    "notarization_required": false,
    "translation_required": true
  }
}
```

```json
{
  "role": "ELIGIBILITY_RULE",
  "subtype": "nationality_restriction",
  "facets": {
    "applies_negatively": true,
    "waivable": false,
    "restriction_type": "visa_category_exclusion"
  }
}
```

```json
{
  "role": "TIMELINE_ITEM",
  "subtype": "advance_booking_window",
  "facets": {
    "direction": "before_appointment",
    "is_calendar_days": true,
    "excludes_weekends": false
  }
}
```

#### Правила facets

- Facet keys — из реестра `kb.facet_keys`, управляемого `ontology_curator`
- Новый facet key добавляется через ontology intake (sec 42.1), а не произвольно
- Facets не создают новые required_fields и не меняют binding contract
- Facets не влияют на graph MERGE logic (они хранятся как node properties, не как отношения)
- Facet с `null` значением допустим — означает "не применимо / не известно"

**ЖЁСТКОЕ ПРАВИЛО:** facets — это enrichment, не структурный контракт. Extraction валидна без facets (при наличии обязательных полей роли). Отсутствие facets не может быть причиной отклонения RuleInstance на Schema Validator.

---

### 44.4 Forbidden Role Expansion Rule

Новая procedural role **запрещена**, если различие может быть выражено через любое из:

- `params`
- `conditions_raw` / `exceptions_raw`
- `alternatives`
- `subtype`
- `facets`
- `applies_to_profiles`

Новая procedural role допустима **только** при выполнении **всех** следующих условий:

1. Меняется **structural graph shape** — требуется новый тип Neo4j узла или новое обязательное отношение
2. Меняется **binding contract** — требуются новые обязательные поля в Role-to-Binding Matrix (sec 35.4)
3. Меняется **serving semantics** — требуется новая ветка отображения или новый тип checklist
4. Ни одна существующая роль + specialization не выражает объект

При выполнении всех 4 условий это не HITL изменение — это **Protocol Revision**, требующая:
- одобрения `protocol_owner`
- обновления sec 35.4, Schema Registry (sec 36.2), Triple Builder, GraphContracts
- migration plan для historic extractions

---

### 44.5 Role Extension Review Checklist

Перед любым предложением новой роли команда обязана ответить на 5 вопросов.

| # | Вопрос | Если YES | Если NO |
|---|---|---|---|
| Q1 | Это реально новый **тип** procedural fact или specialization существующего? | Продолжить → Q2 | Использовать subtype/facet, роль запрещена |
| Q2 | Можно ли выразить через subtype/facet без изменения binding contract? | Новая роль **запрещена** | Продолжить → Q3 |
| Q3 | Меняется ли required binding matrix (новые обязательные поля)? | Продолжить → Q4 | Сильный аргумент против новой роли |
| Q4 | Меняется ли graph relation model (новые Neo4j отношения)? | Продолжить → Q5 | Сильный аргумент против новой роли |
| Q5 | Требуется ли новая serving logic (отдельная ветка display/checklist)? | Рассмотреть Protocol Revision | Новая роль **запрещена** |

**Правило принятия решения:**

- Q2 = YES → новая роль ЗАПРЕЩЕНА немедленно
- Q2 = NO + Q3 = NO + Q4 = NO + Q5 = NO → новая роль ЗАПРЕЩЕНА
- Q2 = NO + хотя бы 2 из Q3/Q4/Q5 = YES → инициировать Protocol Revision
- Один YES из Q3/Q4/Q5 без остальных → использовать subtype/facet + escalate к `protocol_owner` для подтверждения

---

### 44.6 Role Capability Matrix

Расширяет sec 35.4 (binding requirements) семантическими возможностями. Binding columns здесь не повторяются.

| role | requires_concept | requires_params | allows_conditions | allows_exceptions | allows_alternatives | allows_temporal_window | allows_scope_override | subtype_supported | facets_supported |
|---|---|---|---|---|---|---|---|---|---|
| `DOCUMENT_REQUIRED` | yes | yes | yes | yes | yes | no | yes | yes | yes |
| `ELIGIBILITY_RULE` | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| `FEE_ITEM` | yes | yes | yes | yes | yes | no | yes | yes | yes |
| `TIMELINE_ITEM` | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| `WHERE_TO_APPLY` | yes | no | yes | no | yes | no | yes | yes | yes |
| `APPOINTMENT_RULE` | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| `FORM_REQUIRED` | yes | yes | yes | yes | yes | no | yes | yes | yes |
| `STEP` | yes | yes | yes | yes | yes | yes | yes | yes | yes |

**Пояснения:**

- `allows_temporal_window` = роль может содержать семантику временного окна применимости (сверх `params` quantities). У `DOCUMENT_REQUIRED` и `FEE_ITEM` этот смысл выражается через params и versioning, не через отдельный temporal window.
- `WHERE_TO_APPLY` не поддерживает `allows_exceptions` как structural feature — исключения к месту подачи концептуально выражаются через `conditions_raw` или отдельную `WHERE_TO_APPLY` запись с условием.
- `subtype_supported` и `facets_supported` = YES для всех ролей без исключения.

---

## 45. Registry Governance as Operational Subsystem

Sec 42 определяет политики ontology ops. Этот раздел определяет **операционную механику**: как именно registry changes движутся через систему, кто что решает, какой инструментарий это поддерживает.

Без этого governance превращается в ручное горлышко: curator становится bottleneck, alias conflicts копятся, merge/split decisions непрозрачны, ontology начинает отставать от extraction reality.

---

### 45.1 Registry Change Classes

Не все registry changes равны по риску. Класс определяет путь изменения — от fast-path до formal decision.

| Класс | Риск | Примеры | Путь |
|---|---|---|---|
| **A** | Низкий | Новый alias (без коллизии), новый пример в entity definition, typo fix, metadata enrichment, новое значение существующего facet key | Fast-path: авто-валидация, без curator review |
| **B** | Средний | Новый canonical key, новый profile, новый scope facet, новый relation convention, новый subtype, новый facet_key, deprecation alias | Curator review: обязательно |
| **C** | Высокий | Merge canonical keys, split key, deprecation active key с existing verified rules, изменение relation semantics, remap verified rules, profile taxonomy restructuring | Formal decision: protocol_owner sign-off |

**ЖЁСТКОЕ ПРАВИЛО:** класс присваивается автоматически при создании proposal по таблице выше. Ручное понижение класса (Class C → B или B → A) допустимо только `protocol_owner` с явным обоснованием.

---

### 45.2 Change Workflow

Curator workflow — явный state machine. Совместим с intake statuses sec 42.1, добавляет operational routing по классу.

```
proposal_created
        ↓
[auto-triage: assign change class A / B / C]
        │
        ├── Class A ──→ fast_path_validation
        │                    ↓               ↓
        │             validation_passed  validation_failed
        │                    ↓               ↓
        │             auto_approved    ──→ proposed (к curator)
        │                    ↓
        │             registry_applied
        │
        ├── Class B ──→ curator_review
        │                    ↓
        │             approved | rejected | needs_more_info
        │                    ↓ (approved)
        │             registry_applied
        │
        └── Class C ──→ impact_analysis (обязательна, см. 45.4)
                             ↓
                     [impact report ready]
                             ↓
                     formal_decision (protocol_owner sign-off)
                             ↓
                     approved | rejected | deferred
                             ↓ (approved)
                     migration_planned (если complex change)
                             ↓
                     registry_applied
        │
        └── [после registry_applied для всех классов]
                     reindex_required? ──→ reindex_queued
                     graph_repair_required? ──→ graph_repair_queued
                             ↓
                         completed
```

Каждый переход фиксируется в audit log с `actor`, `timestamp`, `justification`.

---

### 45.3 Fast-Path Rules (Class A)

Class A change авто-применяется после прохождения следующих проверок. Все проверки детерминированные — не требуют human review.

| Проверка | Критерий pass |
|---|---|
| Alias uniqueness | Нет collision в том же `entity_type` + `locale` |
| Forbidden aliases check | Alias не в `forbidden_aliases` реестре |
| Structural format | Поля заполнены корректно (тип, locale, key reference) |
| Impact scope | Изменение чисто additive — не затрагивает existing verified rules |
| Ambiguity budget | `ambiguous_aliases` не превысил Warning threshold (sec 42.6) |

При fail любой проверки: Class A upgrade → Class B, направляется к curator с указанием причины.

**ЖЁСТКОЕ ПРАВИЛО:** Fast-path не применяется к alias, который мог бы создать коллизию даже в другом locale, если `entity_type` совпадает. Сомнение — всегда Class B.

---

### 45.4 Impact Analysis Requirements

Обязательна для **Class B** и **Class C** до применения изменения.

#### Class B (Minimum)

| Поле | Описание |
|---|---|
| `affected_verified_rules_count` | Число verified RuleInstances, ссылающихся на изменяемый key/alias |
| `affected_graph_nodes_count` | Число Neo4j узлов, затронутых изменением |
| `retrieval_reindex_required` | bool + список затронутых Voyage/Qdrant коллекций |
| `pipeline_consumers` | Шаги pipeline, потребляющие изменяемый key |

#### Class C (All of Class B, plus)

| Поле | Описание |
|---|---|
| `backward_compatibility_assessment` | Оценка: остаются ли historic extractions валидными |
| `migration_sequence` | Пошаговый план применения изменения |
| `rollback_procedure` | Явный план отмены изменения при неудаче |
| `estimated_affected_batches` | Число batch-операций для remap |
| `graph_migration_preview` | Preview Neo4j изменений до применения |

**ЖЁСТКОЕ ПРАВИЛО:** Class C change без полной impact analysis не достигает `formal_decision`. `protocol_owner` не подписывает decision без impact report.

---

### 45.5 Conflict Ownership

Явная таблица ownership по типу конфликта. Устраняет ситуацию когда "кто-нибудь разберётся".

| Тип конфликта | Первый owner | Эскалация |
|---|---|---|
| Alias collision (один alias → 2+ keys) | `ontology_curator` | → `domain_reviewer` если не разрешимо за SLA |
| Alias → forbidden aliases list | `protocol_owner` | — |
| Semantic merge proposal (2 keys = одна сущность) | `ontology_lead` + `domain_reviewer` | → `protocol_owner` |
| Split proposal (1 key = 2 разные сущности) | `ontology_lead` + `domain_reviewer` | → `protocol_owner` |
| Profile taxonomy change | `domain_reviewer` | → `protocol_owner` |
| Relation convention change | `protocol_owner` | — |
| Новый facet_key в реестре | `ontology_curator` | → `domain_reviewer` если cross-role |
| Subtype restructuring | `ontology_curator` + `domain_reviewer` | → `protocol_owner` |
| Deprecated key remap (active verified rules) | `domain_reviewer` | → `protocol_owner` |
| Graph migration (Class C) | `protocol_owner` | — |

**Правило эскалации:** если первый owner не принял решение в рамках SLA (sec 42.5), конфликт автоматически эскалирует. Ненастроенная эскалация — нарушение governance.

---

### 45.6 Ambiguity Budget per Domain

Дополняет общий ambiguity budget (sec 42.6). Отслеживает drift на уровне конкретного домена (country + visa_type) — чтобы не скрывать локальные проблемы за хорошими общими показателями.

| Метрика | Единица | Warning | Critical |
|---|---|---|---|
| `unresolved_candidates` per domain | per 1000 sections | > 5 | > 15 |
| `ambiguous_aliases` per domain | per domain slice | > 2 | > 5 |
| `temp_fallback_mappings` per domain | per domain slice | > 3 | > 10 |

**Правило блокировки домена:** при достижении Critical по любой per-domain метрике — extraction pass для этого `country + visa_type` блокируется до снижения ниже Warning. Общий budget (sec 42.6) при этом может быть в норме — per-domain блокировка независима.

Это предотвращает ситуацию когда общий budget ОК, а один домен (например, новая страна) деградировал незаметно.

---

### 45.7 Required Tooling

Минимальный набор инструментов без которого governance operations становятся ручным адом. Перечислены по приоритету.

| Инструмент | Функция | Приоритет |
|---|---|---|
| **Registry diff viewer** | Показывает что изменилось между версиями registry (какие keys/aliases добавлены, deprecated, merged) | P0 |
| **Alias collision detector** | Обнаруживает ambiguous aliases до того, как они вызовут extraction errors; запускается при каждом Class A/B change | P0 |
| **Impact analyzer** | Для данного предложенного change показывает: сколько verified rules затронуто, какие graph nodes, нужен ли reindex | P0 |
| **Reindex queue planner** | Составляет план re-indexing Voyage/Qdrant после registry changes с учётом приоритетности domainов | P1 |
| **Merge preview** | Показывает результат merge двух keys до применения: affected rules, graph preview, alias union | P1 |
| **Split analysis tool** | Для split: показывает список RuleInstances требующих re-attribution, формирует `needs_review` batch | P1 |
| **Graph migration preview** | Preview Neo4j изменений от Class C operation: какие ребра добавляются/удаляются | P1 |
| **Audit log viewer** | Полный audit trail всех ontology decisions с фильтрацией по типу, классу, owner, дате | P1 |
| **Ambiguity dashboard** | Real-time view метрик (sec 42.6 + per-domain 45.6) с визуализацией trend | P2 |
| **Ontology KPI report** | Автоматический monthly отчёт по KPIs (sec 42.7) | P2 |

**ЖЁСТКОЕ ПРАВИЛО:** инструменты P0 обязаны существовать до первого production deployment extraction pipeline. Запуск pipeline без P0 tooling — нарушение операционной готовности.

---

## 46. Layer Model vs Orthogonal Dimensions

### 46.0 Layer vs Dimension Rule

**Базовый принцип:** слой и dimension — это разные вещи с разными ролями.

| Концепция | Вопрос на который отвечает | Пример |
|---|---|---|
| **Layer** | КАКОЙ ЭТО ТИП ЗНАНИЯ? | `procedural` — нормативные требования; `operational` — текущее состояние |
| **Dimension** | В КАКОМ СТАТУСЕ, КОНТЕКСТЕ И ГРАНИЦАХ это знание существует? | Это verified или draft? Government source или editorial? Timeless или temporary? |

Layers фиксированы и управляют structural type (sec 43.1). Dimensions — это обязательные свойства КАЖДОГО knowledge-объекта, независимо от его layer.

**ЖЁСТКОЕ ПРАВИЛО:** Новый тип authority, temporal semantics, audience scope или provenance **не создаёт новый layer**. Он расширяет существующую dimension. Давление добавить "6-й слой" является сигналом что отсутствует или недостаточно формализована нужная dimension.

#### Extension Test: Layer или Dimension?

Перед предложением нового слоя обязательно применить три теста:

| Тест | Вопрос | Ответ "да" означает |
|---|---|---|
| **Type test** | Требует ли новый концепт полностью других extraction agents, binding contracts и graph relations? | Возможно new layer — через Protocol Revision |
| **Property test** | Описывает ли новый концепт status, authority, temporality, scope или provenance существующего знания? | Это dimension, не layer |
| **Contamination test** | Потребовали бы существующие layers изменений чтобы ссылаться/линковаться на новый "layer"? | Это dimension, не layer |

Примеры применения:
- "compliance layer" → procedural rules и есть regulatory → `authority_level = statutory` (dimension)
- "historical rules layer" → прошлое состояние procedural → `temporal_status = historical` (dimension)
- "jurisdiction-specific layer" → тот же procedural, но для другой юрисдикции → `audience_scope.jurisdiction_scope` (dimension)
- "user-state layer" → personalization serving, не knowledge type → serving plane + `audience_scope` (dimension + plane)

---

### 46.1 Truth Status

Epistemic статус объекта. Расширяет и заменяет поле `status: "verified|extracted|deprecated"` (sec 6.4) — является его superset с backward compatibility.

| Значение | Описание |
|---|---|
| `draft` | Создан вручную или импортирован; ещё не прошёл extraction pipeline |
| `extracted` | Результат extraction pipeline; ожидает verification |
| `verified` | Прошёл полный verification chain; L4 (sec 35.5) |
| `verified_with_limitations` | Верифицирован, но с явными оговорками (неполные params, низкий confidence, частично confirmed source) |
| `deprecated` | Устарел; сохраняется для audit trail |
| `superseded` | Заменён более свежим/точным объектом; имеет `replaced_by` ссылку |
| `blocked` | Заблокирован для публикации; причина фиксируется (contradiction, range, HITL required) |

**Миграция:** существующий `status = "verified"` → `truth_status = "verified"`. Существующий `status = "deprecated"` → `truth_status = "deprecated"`. `status` остаётся как alias-поле для backward compatibility до окончания migration window.

---

### 46.2 Authority Level

Нормативный вес источника. Отличается от `source_tier` (происхождение) тем, что описывает АВТОРИТЕТНОСТЬ знания, а не его категорию.

| Значение | Описание | Пример |
|---|---|---|
| `statutory` | Закон, регламент, официальный нормативный акт | EU Regulation, national visa law |
| `official_government` | Официальный государственный сайт, инструкция | MFA website, consulate official page |
| `official_partner` | Официальный авторизованный партнёр (VAC) | VFS Global, BLS International official page |
| `institutional_operational` | Операционная информация от институции | Bank processing SLA, medical lab requirements |
| `editorial_verified` | Верифицированный editorial контент | Curated explainer, verified guide |
| `editorial_advisory` | Ненормативный advisory контент | Blog post, forum answer, unverified guide |
| `commercial_claim` | Коммерческое утверждение | Ad copy, sales page |

**ЖЁСТКОЕ ПРАВИЛО:** `authority_level` фиксируется из `deterministic job context` + `source_tier`, не из LLM. LLM не оценивает авторитетность источника.

Downstream reasoning: правила с `authority_level = statutory` при конфликте доминируют над `official_government`; `official_government` доминирует над `official_partner` и т.д.

---

### 46.3 Temporal Status

Временная природа знания. Не описывает когда истекает конкретный документ (это `valid_until` в params) — описывает СЕМАНТИЧЕСКИЙ ТИП временнóй применимости правила.

| Значение | Описание | Пример |
|---|---|---|
| `timeless` | Правило не привязано к временному окну; действует пока не superseded | "Паспорт должен быть действителен 3 месяца" |
| `effective_window` | Правило имеет явные `valid_from` и/или `valid_until` | Новый размер сбора с 1 апреля по 31 декабря |
| `temporary_override` | Временно переопределяет timeless правило | Приостановка записи на 2 недели |
| `recurring` | Повторяется по расписанию | Праздничные закрытия, ежегодные пересмотры |
| `historical` | Больше не в силе; сохраняется как evidence | Старый размер сбора |
| `superseded_by_new_rule` | Явно заменён более новым объектом; имеет `replaced_by` ссылку | Устаревшее требование к документам |

**Взаимодействие с `valid_until`:** `temporal_status` — семантика природы правила; `valid_until` — конкретная дата истечения. Временной объект может иметь `temporal_status = effective_window` И конкретный `valid_until`.

---

### 46.4 Audience Scope

Структурированный envelope применимости объекта. Объединяет scope-поля которые сейчас рассеяны по document в единый contract.

```json
{
  "visa_scope": "pl_tourist_by",
  "profile_scope": "student",
  "jurisdiction_scope": "PL",
  "citizenship_scope": "BY",
  "submission_channel_scope": "vac_submission|consulate_submission|postal|online|any",
  "entry_type_scope": "single|multiple|transit|any"
}
```

| Поле | Источник | Null означает |
|---|---|---|
| `visa_scope` | `deterministic job context` | Применимо ко всем визам страны |
| `profile_scope` | extraction (`applies_to_profiles`) | Применимо ко всем профилям |
| `jurisdiction_scope` | `deterministic job context` (country_code) | Без ограничения юрисдикции |
| `citizenship_scope` | `deterministic job context` (target_citizenship) | Не специфично для гражданства |
| `submission_channel_scope` | extraction (`subtype` + `facets`) | Любой канал подачи |
| `entry_type_scope` | extraction | Любой тип въезда |

**ЖЁСТКОЕ ПРАВИЛО:** `audience_scope` заполняется детерминированно из `deterministic job context` + extraction output. LLM не вычисляет scope — он указывает найденные условия применимости, а Rust Triple Builder проставляет финальный envelope.

---

### 46.5 Source Provenance

Группированный envelope трассируемости объекта. Поля существуют в документе в разных местах (sec 1.3, sec 8.2, sec -2.3); здесь зафиксированы как единый первоклассный contract, обязательный для любого knowledge-объекта.

```json
{
  "source_url": "https://poland.mfa.gov.pl/en/visas/tourist",
  "source_domain": "poland.mfa.gov.pl",
  "source_tier": "government",
  "source_document_type": "official_page|regulation|partner_page|editorial|commercial",
  "crawl_version_id": 42,
  "evidence_section_id": "sec_abc123",
  "discovered_at": "2026-01-15T10:00:00Z",
  "last_verified_at": "2026-03-01T12:00:00Z"
}
```

**ЖЁСТКОЕ ПРАВИЛО:** knowledge-объект без `evidence_section_id` не принимается в Truth Plane (аксиома 6). `source_provenance` envelope обязан быть заполнен полностью для всех объектов со `truth_status = verified` или выше.

---

### 46.6 Canonical Object Envelope

Полный envelope любого knowledge-объекта в системе. Объединяет layer с пятью dimensions.

```json
{
  "node_id": "ri_pl_tourist_by_consular_fee_fee_item",
  "layer": "procedural",
  "layer_version": 1,
  "truth_status": "verified",
  "authority_level": "official_government",
  "temporal_status": "timeless",
  "audience_scope": {
    "visa_scope": "pl_tourist_by",
    "profile_scope": null,
    "jurisdiction_scope": "PL",
    "citizenship_scope": "BY",
    "submission_channel_scope": null,
    "entry_type_scope": null
  },
  "source_provenance": {
    "source_url": "https://poland.mfa.gov.pl/en/visas/tourist",
    "source_domain": "poland.mfa.gov.pl",
    "source_tier": "government",
    "source_document_type": "official_page",
    "crawl_version_id": 42,
    "evidence_section_id": "sec_abc123",
    "discovered_at": "2026-01-15T10:00:00Z",
    "last_verified_at": "2026-03-01T12:00:00Z"
  },
  "confidence": 0.97,
  "created_at": "2026-01-15T10:00:00Z",
  "updated_at": "2026-03-01T12:00:00Z"
}
```

Этот envelope является контрактом для:
- Neo4j node properties (sec 6.4)
- Voyage AI payload (sec 8.2)
- verified storage schema (sec 36.2)
- MERGE contracts (sec 37.2, 37.3, 37.4)

**ЖЁСТКОЕ ПРАВИЛО:** отсутствие любого из пяти dimension полей (`truth_status`, `authority_level`, `temporal_status`, `audience_scope`, `source_provenance`) в объекте со `truth_status = verified` является Schema Validation failure (sec 36.3).

---

## 47. Complexity Tiers

Документ описывает систему полной мощности. Но не всё из него нужно строить сразу — и неправильный порядок реализации опаснее чем неполная реализация.

Complexity Tiers — это явная операционная дисциплина: что является обязательным ядром, что строится при наличии production сигнала, и что никогда не строится до него.

**ЖЁСТКОЕ ПРАВИЛО:** Tier 2+ компоненты **не строятся** до получения production сигнала от Tier 0 и Tier 1. Начало Tier 2 без валидации Tier 1 — нарушение операционной дисциплины.

---

### 47.1 Tier 0 — Non-Negotiable Core

Без этого система не работает. Реализуется первой, без компромиссов.

| Компонент | Что даёт |
|---|---|
| Sectioning (без LLM) | Детерминированная нарезка на section с `section_id` и `content_hash` |
| CAS Gate (BLAKE3 + cheap_diff) | Предотвращает бессмысленный re-extraction при layout-only изменениях |
| Entity Span Detection (Agent #2) | `mention_id`, `char_start`, `char_end` — основа deterministic join |
| Canonical Mapping (symbolic-first) | Exact alias → normalized → regex — без Qdrant на этом шаге |
| Layer Router (Agent #1) | `primary_layer` до любого extraction |
| Procedural Extraction (Agent #3A) | ExtractedRuleCandidate с `params`, `evidence_section_id`, `role` |
| Schema Validator | Rejection non-compliant outputs на входе в verified storage |
| Verified Storage (PostgreSQL) | `extracted.*` и `verified.*` таблицы — Truth Plane |
| Evidence binding | `evidence_section_id` обязателен для каждого факта |
| Canonical Key Registry | `kb.canonical`, `kb.aliases` — источник истины для mapping |

**Готовность Tier 0:** система способна извлекать и хранить верифицированные procedural facts с полной трассируемостью.

---

### 47.2 Tier 1 — Quality & Operational Gates

Строится после Tier 0. Обязателен для production-grade quality. Без него система работает, но не является production-ready.

| Компонент | Что даёт |
|---|---|
| Completeness Judge (Agent #5) | 7 типов потерь + hallucination check |
| Resolution Loop | missing/hallucinated → patch/rerun/HITL → повторный Judge |
| HITL Playbook (sec 38) | Управляемый human review для ambiguous cases |
| Failure State Machine (sec 10.1) | Явные state transitions, запрещённые переходы |
| Operational Extraction (Agent #3B) | office schedules, closures, TTL-bound entities |
| Editorial Extraction (Agent #3C) | topics, pain points, связи с procedural |
| Voyage AI vectorization | `verified_rules`, `editorial_topics` коллекции (только L3+) |
| Neo4j sync (L4 only) | RuleInstance-first graph с MERGE contracts |
| Publish Gate | `required_keys`, `is_range=false`, `status=verified` |
| Page Utility Classifier | Hard gate: `allow_procedural_extraction` |
| Ambiguity budget (sec 42.6) | Ограничение ontology debt |

**Готовность Tier 1:** система является production-ready knowledge extraction engine с полным quality control cycle.

---

### 47.3 Tier 2 — Advanced Reasoning

Строится только при наличии production signal: реальный трафик, реальные вопросы пользователей, измеримые gaps в Tier 1 output.

| Компонент | Когда нужен |
|---|---|
| Contradiction detection graph | Когда появляются конфликтующие правила из разных источников в production |
| Cross-layer reasoning | Когда serving plane требует связок procedural ↔ editorial ↔ operational |
| GDS алгоритмы (WCC, PageRank) | Когда нужна кластеризация тем и ranking страниц |
| Qdrant для canonical mapping | Когда symbolic matching даёт < 70% auto-resolution rate |
| Per-domain ambiguity budget (sec 45.6) | Когда появляется ≥ 3 активных domain slice |
| Subtype/facet model (sec 44.2–44.3) | Когда 8 ролей создают реальные retrieval quality проблемы |

**Сигнал для старта Tier 2:** измеримое ухудшение precision/recall в Tier 1 output, которое нельзя устранить улучшением Tier 1 компонентов.

---

### 47.4 Tier 3 — Operational Intelligence

Строится только при масштабе: ≥ 10 активных доменов (country+visa_type) или ≥ 100K verified RuleInstances.

| Компонент | Когда нужен |
|---|---|
| Ontology KPI dashboard (sec 42.7) | При масштабе registry > 1000 canonical keys |
| Per-domain Complexity Report | При ≥ 5 доменах с разными ontology profiles |
| Automated coverage audit | При объёме crawls > 500 страниц в неделю |
| Registry diff tooling P1 (sec 45.7) | При ≥ 3 активных `ontology_curator` одновременно |
| Graph migration preview (sec 45.7) | При первом Class C registry change в production |
| Serving plane analytics | Когда Q&A или CMS output начинает измеряться по user satisfaction |

**Сигнал для старта Tier 3:** операционная сложность registry management или graph maintenance начинает потреблять > 20% engineering time.

---

### 47.5 Do Not Build Rule

| Что не строить без сигнала | Почему |
|---|---|
| Qdrant для canonical mapping до Tier 1 production | Symbolic matching закрывает ≥ 70% случаев; Qdrant добавляет latency и complexity без измеримого benefit |
| Contradiction graph без реальных contradictions | Нет смысла строить detection до появления реального signal |
| Subtype/facet layer до Tier 1 stability | 8 frozen roles достаточны для Tier 0–1; преждевременная specialization создаёт ontology debt |
| Cross-layer GDS reasoning до Neo4j наполнения | PageRank на 100 узлах бессмысленен |
| Per-domain ambiguity monitoring до ≥ 3 доменов | Метрика не даёт сигнала при малом количестве доменов |
| Ontology ops tooling P2 (sec 45.7) до Tier 1 | Dashboard без данных — это потраченное время |

**ЖЁСТКОЕ ПРАВИЛО:** решение "давайте сразу сделаем правильно" без production сигнала — это не engineering discipline, это over-architecture. Tier 0 и Tier 1 — достаточное условие для первого production deployment.

---
