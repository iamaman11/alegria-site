# Proto Contracts

Generated file. Do not edit manually.

## Inventory

| Proto | Package | Messages | Services | SHA256 |
|---|---|---:|---:|---|
| `app/contracts/proto/condition.proto` | `alegria.condition.v1` | 2 | 0 | `6edc65f85dbf94426e150f0d997fe4f98d4bdba5a80b4afb863b850d3447369c` |
| `app/contracts/proto/ontology.proto` | `alegria.ontology.v1` | 6 | 0 | `dce4dff6cc54bd745ec112f68b24c17495d0190fd7117d89255c0470daad9368` |
| `app/contracts/proto/primitives.proto` | `alegria.primitives.v1` | 10 | 1 | `1eab30267de1ce980ce3e37e84893fb3f8b7fe21a03e7204663e6cf1db94b34d` |
| `app/contracts/proto/read_api.proto` | `alegria.read_api.v1` | 14 | 0 | `a690559a168b00b7ed554002e84ddad902ed346d989f82a068953af729ca4561` |
| `app/contracts/proto/rules.proto` | `alegria.rules.v1` | 11 | 0 | `8eb0f733a0929558c39b0359b583b507f484ad614ee25f74a55eaa40e2601434` |
| `app/contracts/proto/sync.proto` | `alegria.sync.v1` | 12 | 0 | `7e526dd92504423f29fafaabe53efb07a14be74cfde7fccf59e966e1cbc017b0` |
| `app/contracts/proto/telemetry.proto` | `alegria.telemetry.v1` | 2 | 0 | `4924ffcaad50a1d1e7b8f97e68caa857b876905c6683f3b12616d2a2bbe45f78` |
| `app/contracts/proto/temporal_payloads.proto` | `alegria.temporal.v1` | 94 | 0 | `0c5d378484da775ff8b6eaffd23ce9993411281dd2e939bd432dbf000a7e3456` |
| `app/analytics_lab/proto/analytics.proto` | `alegria.analytics.v1` | 10 | 1 | `3e3b8b852b9a46fd656d7e14e61f23c2d161f29dbaf5d3281ea760dcbf3ad21e` |

## Message/Service Index

### `app/contracts/proto/condition.proto`

- messages: ConditionExprV1, ScalarValue
- services: —

### `app/contracts/proto/ontology.proto`

- messages: ApplicantProfile, Concept, ConceptHierarchyEdge, PageContextMap, Source, VisaContext
- services: —

### `app/contracts/proto/primitives.proto`

- messages: HashRequest, HashResponse, NormalizeRequest, NormalizeResponse, PingRequest, PingResponse, StableIdRequest, StableIdResponse, UrlRequest, UrlResponse
- services: PrimitivesService

### `app/contracts/proto/read_api.proto`

- messages: AppointmentRuleParams, CitationFact, ContextBundle, ContextBundleRequest, DocumentRequiredParams, EligibilityRuleParams, FeeItemParams, FormRequiredParams, OntologyRuleView, RelatedLink, SourceCitation, StepParams, TimelineItemParams, WhereToApplyParams
- services: —

### `app/contracts/proto/rules.proto`

- messages: AppointmentRuleParams, DocumentRequiredParams, EligibilityRuleParams, FeeItemParams, FormRequiredParams, RuleException, RuleInstance, RuleProfileApplicability, StepParams, TimelineItemParams, WhereToApplyParams
- services: —

### `app/contracts/proto/sync.proto`

- messages: ConceptApproved, ConceptHierarchyChanged, Neo4jMaterializationCommand, Neo4jRuleUpsertPayload, PageContextMapped, QdrantEntityPayload, QdrantUpsertCommand, RuleInstanceDeprecated, RuleInstanceUpserted, SeoCmsEventPayload, SeoGraphProjectionPayload, SyncOutboxEvent
- services: —

### `app/contracts/proto/telemetry.proto`

- messages: RuntimeSyncStatus, WorkerHeartbeat
- services: —

### `app/contracts/proto/temporal_payloads.proto`

- messages: CannibalizationConflictState, ClaimLedgerEntry, CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload, CmsReviewPage, ContentBlockPlanItemState, ContentBlockPlanState, ContentContractValidateInputPayload, ContentContractValidateOutputPayload, ContentGapState, CrawlSourcesInputPayload, CrawlSourcesOutputPayload, DomBlockRelevanceDecisionPayload, DomBlockRelevancePayload, DraftAssembleInputPayload, DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload, DraftQaInputPayload, DraftQaOutputPayload, DraftState, EditorialBrief, EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload, ExtractedPayloadState, FactExtractionInputPayload, FactValueState, FinalizePublishInputPayload, FinalizePublishOutputPayload, FreshnessReport, GenerationBlockState, GenerationResultState, GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, HitlDecision, HitlPauseInfo, HitlResolutionInput, HitlTaskContext, IaBuildInputPayload, IaBuildOutputPayload, KeywordClusterState, LinkRecommendInputPayload, LinkRecommendOutputPayload, LinkRecommendationState, LlmDraftCandidate, LlmDraftRequest, OpportunityBuildInputPayload, OpportunityBuildOutputPayload, PageBlueprintState, PageBriefState, PageNodeState, PageSemanticContextPayload, PageUtilityClassificationPayload, PersistReport, PublishArtifact, PublishMaterializeInputPayload, PublishMaterializeOutputPayload, QualityPolicyEvaluationInputPayload, QualityPolicyEvaluationOutputPayload, RawKnowledgeIngestionInputPayload, RawKnowledgeIngestionOutputPayload, RebuildDetectInputPayload, RebuildDetectOutputPayload, RebuildImpactState, ReconcileSummaryPayload, ReconcileTargetReportPayload, RenderPreviewPageState, RenderPreviewValidateInputPayload, RenderPreviewValidateOutputPayload, RenderedContentBlock, RuleInstanceCandidateState, RuleParamsState, RuntimeErrorPayload, SectionTemplateBinding, SeoDraftSectionState, SeoPublishBlockerState, SeoScopePayload, SeoSiteBuildInputPayload, SeoTraceabilityEntryState, SeoVerifiedFactSupportState, SerpIngestInputPayload, SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload, SerpPatternState, SourceContextChunkState, StepContractMeta, StepEnvelope, StringPayload, ValidationDiagnostic, ValidationInputPayload, ValidationReport, VerifyReport
- services: —

### `app/analytics_lab/proto/analytics.proto`

- messages: LinkEntry, LinkPlanRequest, LinkPlanResult, PageRankEntry, PageRankRequest, PageRankResult, PingRequest, PingResponse, WCCRequest, WCCResult
- services: GraphAnalyticsService
