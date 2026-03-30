// Normative semantic projection contract
// Includes exact nodes and edges required for ContextBundle assembly and Q&A.

CALL gds.graph.project(
  'visaOntology',
  ['VisaContext', 'Concept', 'ApplicantProfile'],
  {
    DOCUMENT_REQUIRED: {orientation: 'DIRECTED'},
    ELIGIBILITY_RULE:  {orientation: 'DIRECTED'},
    FEE_ITEM:          {orientation: 'DIRECTED'},
    TIMELINE_ITEM:     {orientation: 'DIRECTED'},
    WHERE_TO_APPLY:    {orientation: 'DIRECTED'},
    APPOINTMENT_RULE:  {orientation: 'DIRECTED'},
    FORM_REQUIRED:     {orientation: 'DIRECTED'},
    STEP:              {orientation: 'DIRECTED'},
    IS_A:              {orientation: 'DIRECTED'},
    APPLIES_TO:        {orientation: 'DIRECTED'}
  }
)
// NOTE: Excludes HAS_RULE, CONCERNS, SUPPORTED_BY, REPRESENTS to avoid polluting 
// graph similarity algorithms with the structural dual-layer noise.
