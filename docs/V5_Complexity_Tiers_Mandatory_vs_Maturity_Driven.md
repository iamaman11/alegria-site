# Complexity Tiers: Mandatory vs Maturity-Driven

## Зачем нужен этот раздел

Система в текущем виде архитектурно очень сильная, но именно поэтому у неё есть риск утонуть в собственной мощности. В протоколе уже описаны не только обязательные инварианты извлечения, но и зрелые слои sophistication: contradiction handling, dependency-driven re-extraction, graph reasoning, GDS-проекции, ontology operations, curator SLA и другие расширения.

Если не зафиксировать границу между **core complexity** и **maturity-driven complexity**, команда почти неизбежно начнёт строить всё сразу. Это создаст ложную зависимость между запуском ядра и функциями, которые на самом деле нужны только после появления сигнала из эксплуатации.

Поэтому протокол должен явно разделять:

- то, без чего концепция теряет смысл;
- то, что сильно улучшает качество, но не блокирует первый запуск;
- то, что является advanced intelligence layer;
- то, что относится к scale/optimization и не должно навязываться стартовой реализации.

---

## Core Rule

Не вся сложность одинаково обязательна.

Система обязана различать:

1. **Core mandatory** — без этого протокол теряет идентичность.
2. **Strongly recommended** — это уже очень полезно, но ранний запуск без этого возможен.
3. **Advanced intelligence** — это не launch core, а sophistication layer.
4. **Scale optimization** — это появляется при росте нагрузки, команды и объёма истины.

Из этого следует правило проектирования:

> Наличие архитектурной идеи в протоколе не означает обязательность её немедленной реализации.

---

## 6.1. Tier 0 — обязательное ядро

Это то, без чего концепция V5 фактически перестаёт быть самой собой.

### Что входит в Tier 0

- `sectioning`
- `layer routing`
- `mentions`
- `canonical mapping`
- `procedural truth extraction`
- `evidence binding`
- `schema validation`
- `verification gates`
- `deterministic storage boundaries`

### Почему это ядро

Именно этот набор создаёт минимально осмысленную систему:

- страница режется на управляемые semantic units;
- слой определяется до extraction;
- факт не возникает без mention и evidence;
- canonical mapping не даёт системе тонуть в свободном тексте;
- procedural truth extraction выделяет то, ради чего строится весь pipeline;
- schema validation и verification gates отделяют сырое извлечение от пригодного знания;
- deterministic storage boundaries не дают смешивать уровни готовности (`extracted`, `verified`, retrieval-ready, graph-safe).

### Правило

Tier 0 реализуется обязательно до любого разговора о расширенной интеллектуальности системы.

Если Tier 0 не завершён, любые инвестиции в graph intelligence, curator tooling или advanced editorial logic считаются преждевременными.

---

## 6.2. Tier 1 — сильно желательное

Это то, что уже даёт заметный прирост качества и надёжности, но не является жёстким условием первого запуска.

### Что входит в Tier 1

- `completeness judge`
- `null-canonical guard`
- `role binding matrix`
- `HITL basic flow`
- `verified vector indexing`
- `operational lifecycle rules`

### Почему это Tier 1, а не Tier 0

Эти элементы не определяют базовую идентичность протокола, но резко повышают его эксплуатационную зрелость:

- `completeness judge` уменьшает semantic loss;
- `null-canonical guard` не даёт протащить опасную полу-канонизацию в downstream;
- `role binding matrix` делает graph-safe storage более предсказуемым;
- `HITL basic flow` создаёт контролируемый выход из ambiguity;
- `verified vector indexing` делает verified knowledge пригодным для retrieval;
- `operational lifecycle rules` вводят TTL и refresh discipline там, где данные стареют.

### Правило

Tier 1 желательно строить сразу после стабилизации Tier 0.

Но отсутствие Tier 1 не должно блокировать ранний запуск, если:

- Tier 0 инварианты соблюдены;
- publication boundaries не нарушаются;
- unresolved ambiguity не попадает в verified plane.

---

## 6.3. Tier 2 — advanced intelligence

Это уже не ядро запуска, а слой интеллектуального усложнения. Он полезен, но не должен подменять собой core implementation.

### Что входит в Tier 2

- `cross-layer graph reasoning`
- `contradiction graph`
- `complex dedup / contradiction workflows`
- `GDS algorithms`
- `editorial sophistication`
- `semantic similarity over topic graph`

### Почему это Tier 2

Эти возможности начинают приносить максимальную пользу только после того, как:

- базовые сущности уже стабильно извлекаются;
- verified plane уже существует;
- canonical layer уже управляем;
- накоплен достаточный объём truth objects и связей.

До этого момента advanced graph reasoning почти всегда опирается на слишком слабое основание и превращается в дорогую интеллектуализацию поверх ещё нестабильного ядра.

### Риск преждевременной реализации

Если начать строить Tier 2 слишком рано:

- команда будет оптимизировать то, что ещё не стабилизировалось;
- contradiction workflows станут компенсировать сырость Tier 0 вместо того, чтобы исправлять реальные противоречия;
- GDS и similarity создадут видимость sophistication без гарантии качества базовой истины;
- editorial layer начнёт усложняться раньше, чем serving plane научится системно потреблять его результат.

### Правило

Tier 2 допускается только после того, как Tier 0 стабилен, а Tier 1 закрывает основные quality gaps.

---

## 6.4. Tier 3 — scale/optimization

Это то, что нужно не на старте, а при росте продукта, команды и объёма знаний.

### Что входит в Tier 3

- `advanced ontology ops dashboards`
- `graph analytics for optimization`
- `automated drift reporting`
- `domain hot-spot mining`
- `curator productivity tooling`

### Почему это Tier 3

Этот слой нужен, когда система уже доказала свою полезность и упёрлась в operational scale:

- ontology ops начинают требовать отдельной обзорности;
- граф используется достаточно интенсивно, чтобы аналитика реально влияла на решения;
- drift становится системной проблемой, а не редким кейсом;
- curator throughput превращается в ограничение продукта;
- hot-spot mining нужен для приоритизации развития доменов, а не для proof of concept.

### Правило

Tier 3 запрещено считать prerequisite для первого production-grade запуска.

Это optimisation layer, а не launch layer.

---

## 6.5. “Do not build before signal” rule

Для maturity-driven сложности вводится отдельное правило:

> Система не строит дорогой слой заранее только потому, что он архитектурно красив. Такой слой реализуется только после появления явного эксплуатационного сигнала.

### Что считается сигналом

Сигналом считается не абстрактное желание «сделать систему умнее», а конкретный наблюдаемый дефицит:

- повторяющийся тип ошибки;
- ограничение качества, которое нельзя закрыть Tier 0/Tier 1 средствами;
- накопившийся объём объектов или конфликтов;
- явный потребитель результата в serving plane;
- измеримый bottleneck команды или пайплайна.

### Примеры применения правила

- **GDS algorithms** не реализуются до доказанной потребности в graph analytics или path-based reasoning.
- **Contradiction graph** не строится полностью, пока не появился достаточный объём conflicting truth.
- **Editorial sophistication** не усложняется, пока serving plane не начал использовать это системно.
- **Advanced ontology ops dashboards** не строятся, пока ручная curator-операция реально не стала узким местом.
- **Automated drift reporting** не внедряется как полноценный subsystem, пока drift не стал регулярной operational проблемой.

### Практический смысл

Это правило защищает систему от трёх типовых ошибок:

1. **Архитектурная жадность** — желание реализовать весь красивый дизайн сразу.
2. **Ложная обязательность** — когда optional sophistication начинает восприниматься как core dependency.
3. **Скрытая задержка запуска** — когда реализация зрелых слоёв незаметно блокирует production readiness ядра.

---

## Что считается дорогой сложностью

Под **дорогой сложностью** в рамках протокола понимается не просто объём кода, а комбинация нескольких факторов:

- функция не нужна для базовой идентичности системы;
- функция требует зрелого upstream quality, иначе даёт слабый результат;
- функция требует накопленного объёма graph/truth/operations data;
- функция увеличивает объём HITL, governance или поддержки;
- функция добавляет новые контуры принятия решений, а не только усиливает существующий pipeline;
- функция не даёт немедленного launch-critical эффекта.

Если компонент удовлетворяет нескольким из этих признаков, он должен по умолчанию рассматриваться как maturity-driven, а не core.

---

## Implementation Policy

При планировании работ команда обязана помечать каждый крупный компонент одним из четырёх статусов:

- `T0_core_mandatory`
- `T1_strongly_recommended`
- `T2_advanced_intelligence`
- `T3_scale_optimization`

### Обязательное правило планирования

Нельзя:

- поднимать задачу из Tier 2 или Tier 3 в статус blocking без отдельного обоснования;
- объявлять Tier 2/Tier 3 частью MVP по умолчанию;
- строить новый sophistication layer без зафиксированного сигнала;
- компенсировать недостроенный Tier 0 за счёт роста Tier 2.

### Разрешено

Разрешено закладывать архитектурные точки расширения под будущие уровни зрелости, если это:

- не блокирует запуск;
- не создаёт немедленной сложности в runtime;
- не заставляет команду реализовывать optional layer заранее.

Иными словами: **архитектурная готовность допустима; преждевременная реализация — нет**.

---

## Что это даст

### 1. Продукт не утонет в complexity

Команда перестанет воспринимать весь протокол как единый монолит обязательной реализации.

### 2. Архитектура сохранится, но внедрение станет реалистичным

Полная архитектурная картина остаётся в протоколе, но появляется порядок зрелости, а не требование построить всё одновременно.

### 3. Появится ясная дорожная карта зрелости

Станет видно:

- что нужно для запуска;
- что нужно для повышения качества;
- что нужно для advanced intelligence;
- что нужно только при росте масштаба.

### 4. Снизится риск ложных блокеров

Команда не будет тормозить Tier 0 из-за отсутствия Tier 2/Tier 3 возможностей.

### 5. Упростится принятие product/engineering решений

Любую спорную инициативу можно будет проверить через один вопрос:

> Это обязательное ядро или maturity-driven сложность?

---

## Предлагаемая вставка в основной протокол

Ниже формулировка, которую можно вставить в основной документ как отдельный раздел.

---

## Complexity Tiers: Mandatory vs Maturity-Driven

Система ОБЯЗАНА явно различать обязательную сложность и maturity-driven сложность.

Не каждый описанный в протоколе механизм является частью launch core.

### Tier 0 — Core mandatory

Это обязательное ядро, без которого концепция протокола теряет смысл.

Сюда входят:

- sectioning
- layer routing
- mentions
- canonical mapping
- procedural truth extraction
- evidence binding
- schema validation
- verification gates
- deterministic storage boundaries

### Tier 1 — Strongly recommended

Это сильно желательные механизмы, которые заметно повышают качество и надёжность, но не блокируют ранний запуск.

Сюда входят:

- completeness judge
- null-canonical guard
- role binding matrix
- HITL basic flow
- verified vector indexing
- operational lifecycle rules

### Tier 2 — Advanced intelligence

Это sophistication layer, а не обязательное ядро запуска.

Сюда входят:

- cross-layer graph reasoning
- contradiction graph
- complex dedup / contradiction workflows
- GDS algorithms
- editorial sophistication
- semantic similarity over topic graph

### Tier 3 — Scale optimization

Это механизмы, нужные при росте продукта, а не на старте.

Сюда входят:

- advanced ontology ops dashboards
- graph analytics for optimization
- automated drift reporting
- domain hot-spot mining
- curator productivity tooling

### Do not build before signal

Tier 2 и Tier 3 не реализуются заранее только потому, что они предусмотрены архитектурой.

Они строятся только после появления доказанного сигнала из эксплуатации:

- повторяющийся тип ошибки;
- накопленный объём conflicting truth;
- измеримый bottleneck;
- потребность serving plane;
- подтверждённый scale effect.

ЖЁСТКОЕ ПРАВИЛО:

- отсутствие Tier 2/Tier 3 не может считаться блокером запуска Tier 0;
- optional sophistication не может объявляться mandatory без отдельного обоснования;
- зрелые механизмы не должны компенсировать недостроенное ядро.

---

## Итоговое правило

Сначала система должна стать **правильной**, потом **надёжной**, потом **умной**, и только потом **оптимизированной под масштаб**.

Именно в таком порядке complexity остаётся управляемой.
