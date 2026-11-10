#!/usr/bin/env bash
# Read-only psql evidence for R5: keep SQL out of the Python orchestration.
# This script must never seed, update, delete, restore or approve data.
set -euo pipefail

country="${ALEGRIA_CERT_COUNTRY_CODE:-ES}"
visa_type="${ALEGRIA_CERT_VISA_TYPE:-tourist}"
citizenship="${ALEGRIA_CERT_CITIZENSHIP_CODE:-BY}"
for value in "$country" "$visa_type" "$citizenship"; do
  if [[ ! "$value" =~ ^[a-zA-Z0-9_-]{2,32}$ ]]; then
    echo "invalid scope identifier for fact snapshot" >&2
    exit 2
  fi
done

sql="
SELECT coalesce(jsonb_object_agg(
    r.rule_instance_id,
    jsonb_build_object(
      'params', r.params,
      'evidence_quote', r.evidence_quote,
      'source_snapshot_hash', r.source_snapshot_hash,
      'source_key', r.source_key
    )
), '{}'::jsonb)
FROM verified.rule_instances r
WHERE r.status='verified'
  AND r.publish_admissibility='admissible'
  AND r.context_key IN (
    SELECT context_key FROM kb.visa_contexts
    WHERE upper(country_code)=upper('$country')
      AND lower(visa_family)=lower('$visa_type')
      AND upper(citizenship_code)=upper('$citizenship')
      AND (visa_subtype IS NULL OR visa_subtype='')
      AND status='active'
  );
"

docker compose exec -T postgres sh -ec \
  'PGPASSWORD="$POSTGRES_PASSWORD" exec psql -X -A -t -U postgres -d alegria -c "$1"' \
  sh "$sql"
