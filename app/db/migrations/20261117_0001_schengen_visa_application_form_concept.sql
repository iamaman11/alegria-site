BEGIN;
SET LOCAL lock_timeout = '5s';

-- The official Schengen application form is a documented concept, not an
-- inferred VERIFIED visa rule. Existing rule/evidence rows are untouched.
INSERT INTO kb.concepts (concept_key, concept_type, label_ru, status)
VALUES ('schengen_visa_application_form', 'document', 'Анкета на шенгенскую визу', 'active')
ON CONFLICT (concept_key) DO NOTHING;

COMMIT;