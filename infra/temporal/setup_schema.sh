#!/bin/sh
# Temporal's supported admin-tools schema bootstrap (PostgreSQL 12+ plugin).
# Idempotent on repeated compose up: keep real database errors fail-closed.
set -eu
: "${POSTGRES_SEEDS:?POSTGRES_SEEDS must be configured}"
: "${POSTGRES_USER:?POSTGRES_USER must be configured}"
: "${SQL_PASSWORD:?SQL_PASSWORD must be configured}"
port="${DB_PORT:-5432}"
nc -z -w 10 "$POSTGRES_SEEDS" "$port"

db_tool() {
  temporal-sql-tool --plugin postgres12 --ep "$POSTGRES_SEEDS" \
    -u "$POSTGRES_USER" -p "$port" --db "$1" "$2" ${3:+"$3"} ${4:+"$4"}
}
for db in temporal temporal_visibility; do
  case "$db" in
    temporal) kind=temporal ;;
    temporal_visibility) kind=visibility ;;
  esac
  if ! output="$(db_tool "$db" create 2>&1)"; then
    case "$output" in
      *"already exists"*) : ;;
      *) printf '%s\n' "Temporal schema database creation failed: $db" >&2; exit 1 ;;
    esac
  fi
  if ! output="$(db_tool "$db" setup-schema -v 0.0 2>&1)"; then
    case "$output" in
      *"already initialized"*|*"already exists"*) : ;;
      *) printf '%s\n' "Temporal schema initialization failed: $db" >&2; exit 1 ;;
    esac
  fi
  temporal-sql-tool --plugin postgres12 --ep "$POSTGRES_SEEDS" \
    -u "$POSTGRES_USER" -p "$port" --db "$db" update-schema \
    -d "/etc/temporal/schema/postgresql/v12/$kind/versioned"
done
echo "TEMPORAL_SCHEMAS: OK"
