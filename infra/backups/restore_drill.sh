#!/usr/bin/env bash
# Recoverability drill: restore both real dumps into TEMPORARY databases only.
# Never drop a schema in the running Alegria/Temporal databases.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
stamp="$(date -u +%Y%m%d%H%M%S)_$$_$RANDOM"
business_db="alegria_restore_drill_$stamp"
temporal_db="temporal_restore_drill_$stamp"
business_created=0
temporal_created=0

cleanup() {
  status=$?
  trap - EXIT
  set +e
  if [ "$temporal_created" = 1 ]; then
    docker exec alegria_postgres_temporal dropdb -U temporal --if-exists --force "$temporal_db" >/dev/null
    [ $? -eq 0 ] || echo "[restore_drill][WARN] failed to remove temporary Temporal DB $temporal_db" >&2
  fi
  if [ "$business_created" = 1 ]; then
    docker exec alegria_postgres dropdb -U postgres --if-exists --force "$business_db" >/dev/null
    [ $? -eq 0 ] || echo "[restore_drill][WARN] failed to remove temporary business DB $business_db" >&2
  fi
  exit "$status"
}
trap cleanup EXIT

business_dump="$("$ROOT_DIR/infra/backups/backup_business_pg.sh")"
temporal_dump="$("$ROOT_DIR/infra/backups/backup_temporal_pg.sh")"
[ -s "$business_dump" ] && [ -s "$temporal_dump" ] || {
  echo "[restore_drill][FAIL] empty backup dump" >&2; exit 1;
}

# Creating separate databases in EXISTING Postgres containers is non-destructive.
docker exec alegria_postgres createdb -U postgres -O postgres "$business_db"
business_created=1
docker exec alegria_postgres_temporal createdb -U temporal -O temporal "$temporal_db"
temporal_created=1
docker exec -i alegria_postgres psql -X -v ON_ERROR_STOP=1 -U postgres -d "$business_db" < "$business_dump" >/dev/null
docker exec -i alegria_postgres_temporal psql -X -v ON_ERROR_STOP=1 -U temporal -d "$temporal_db" < "$temporal_dump" >/dev/null

schema_sql="SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%';"
business_rows_sql="SELECT count(*) FROM pipeline.execution_runs UNION ALL SELECT count(*) FROM system.sync_outbox UNION ALL SELECT count(*) FROM site.cms_pages;"
temporal_rows_sql="SELECT count(*) FROM public.namespaces UNION ALL SELECT count(*) FROM public.executions;"

read_sql() {
  local container="$1" user="$2" database="$3" sql="$4"
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U "$user" -d "$database" -t -A -c "$sql"
}
compare() {
  local label="$1" source="$2" restored="$3"
  if [ "$source" != "$restored" ]; then
    echo "[restore_drill][FAIL] $label does not match source (source=$source restored=$restored)" >&2
    return 1
  fi
  echo "[restore_drill][PASS] $label=$restored"
}
biz_source="$(read_sql alegria_postgres postgres alegria "$schema_sql")"
biz_restored="$(read_sql alegria_postgres postgres "$business_db" "$schema_sql")"
tm_source="$(read_sql alegria_postgres_temporal temporal temporal "$schema_sql")"
tm_restored="$(read_sql alegria_postgres_temporal temporal "$temporal_db" "$schema_sql")"
[ "$biz_source" -gt 0 ] && [ "$tm_source" -gt 0 ] || {
  echo "[restore_drill][FAIL] source databases have no user tables" >&2; exit 1;
}
compare "business_schema_tables" "$biz_source" "$biz_restored"
compare "temporal_schema_tables" "$tm_source" "$tm_restored"
compare "business_row_counts" \
  "$(read_sql alegria_postgres postgres alegria "$business_rows_sql")" \
  "$(read_sql alegria_postgres postgres "$business_db" "$business_rows_sql")"
compare "temporal_row_counts" \
  "$(read_sql alegria_postgres_temporal temporal temporal "$temporal_rows_sql")" \
  "$(read_sql alegria_postgres_temporal temporal "$temporal_db" "$temporal_rows_sql")"

echo "[restore_drill] PASS: two isolated restores; live databases untouched"
