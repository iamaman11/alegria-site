"""Fail-closed, offline checks for #94's sixteen post-0015 tenant tables.

These checks only protect the *read-only DBA audit* from becoming incomplete or
mutating; they are NOT a substitute for executing that audit against an actual
staging database with FORCE RLS and real roles.
"""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1]
AUDIT = ROOT / "automation" / "pg_projection_force_rls_readiness_v1.sql"
SQL_0021 = ROOT / "app" / "db" / "migrations" / "20261125_0021_pg_vector_projection_v1.sql"
MIGRATIONS = ROOT / "app" / "db" / "migrations"

# Exactly one source-of-truth mapping for the supplemental read-only audit.
# The third component is the issuing migration number, not another SQL path.
NEW_TENANT_POLICIES = {
    "pipeline.qdrant_projection_v2_admissions": (("qdrant_v2_admission_tenant",), "0017"),
    "pipeline.qdrant_projection_v2_attempts": (("qdrant_v2_attempt_tenant",), "0017"),
    "pipeline.qdrant_projection_v2_receipts": (("qdrant_v2_receipt_tenant",), "0017"),
    "pipeline.qdrant_projection_business_key_reservations_v3": (("qdrant_business_reservation_v3_tenant",), "0019"),
    "pipeline.qdrant_projection_v3_admissions": (("qdrant_v3_admission_tenant",), "0019"),
    "pipeline.qdrant_projection_v3_attempts": (("qdrant_v3_attempt_tenant",), "0019"),
    "pipeline.qdrant_projection_v3_receipts": (("qdrant_v3_receipt_tenant",), "0019"),
    "pipeline.operation_dag_run_governance_v1": (("operation_dag_run_governance_v1_tenant",), "0020"),
    "pipeline.vector_projection_sources_v1": (("vector_projection_source_authenticated_tenant_v1", "vector_projection_source_authorized_tenant_insert_v1"), "0021"),
    "pipeline.pg_vector_projection_admissions_v1": (("pg_vector_projection_admission_tenant_v1", "pg_vector_projection_admission_issuer_v1"), "0021"),
    "pipeline.operation_artifact_receipts_v1": (("operation_artifact_receipt_tenant_v1",), "0021"),
    "pipeline.operation_dag_business_runs_v1": (("operation_dag_business_run_tenant_v1",), "0022"),
    "pipeline.operation_dag_run_governance_v2": (("operation_dag_run_governance_v2_tenant",), "0022"),
    "pipeline.qdrant_projection_v4_admissions": (("qdrant_v4_admission_tenant",), "0023"),
    "pipeline.qdrant_projection_v4_attempts": (("qdrant_v4_attempt_tenant",), "0023"),
    "pipeline.qdrant_projection_v4_receipts": (("qdrant_v4_receipt_tenant",), "0023"),
}

EXPECTED_GUARDS = (
    ("pipeline.qdrant_projection_v2_admissions", "guard_qdrant_projection_v2_admission"),
    ("pipeline.qdrant_projection_v2_attempts", "deny_qdrant_projection_v2_attempt_rewrite"),
    ("pipeline.qdrant_projection_v2_receipts", "deny_qdrant_projection_v2_receipt_rewrite"),
    ("pipeline.qdrant_projection_business_key_reservations_v3", "deny_qdrant_business_key_reservation_update_v3"),
    ("pipeline.qdrant_projection_business_key_reservations_v3", "deny_qdrant_business_key_reservation_truncate_v3"),
    ("pipeline.qdrant_projection_v1_admissions", "reserve_qdrant_v1_business_key_v3"),
    ("pipeline.qdrant_projection_v2_admissions", "reserve_qdrant_v2_business_key_v3"),
    ("pipeline.qdrant_projection_v3_admissions", "guard_qdrant_projection_v3_admission"),
    ("pipeline.qdrant_projection_v3_admissions", "reserve_qdrant_v3_business_key_v3"),
    ("pipeline.qdrant_projection_v3_attempts", "deny_qdrant_projection_v3_attempt_rewrite"),
    ("pipeline.qdrant_projection_v3_receipts", "deny_qdrant_projection_v3_receipt_rewrite"),
    ("pipeline.operation_dag_run_governance_v1", "guard_operation_dag_run_governance_v1"),
    ("pipeline.operation_dag_run_governance_v1", "guard_operation_dag_run_governance_truncate_v1"),
    ("pipeline.operation_dag_run_governance_v1", "guard_v1_run_against_active_v2"),
    ("pipeline.operation_dag_business_runs_v1", "guard_operation_dag_business_run_v1"),
    ("pipeline.operation_dag_business_runs_v1", "guard_operation_dag_business_run_truncate_v1"),
    ("pipeline.operation_dag_run_governance_v2", "guard_operation_dag_run_governance_v2"),
    ("pipeline.operation_dag_run_governance_v2", "guard_operation_dag_run_governance_truncate_v2"),
    ("pipeline.qdrant_projection_v4_admissions", "guard_qdrant_projection_v4_admission"),
    ("pipeline.qdrant_projection_v4_admissions", "reserve_qdrant_v4_business_key"),
    ("pipeline.qdrant_projection_v4_admissions", "deny_qdrant_v4_admission_truncate"),
    ("pipeline.qdrant_projection_v4_attempts", "deny_qdrant_v4_attempt_rewrite"),
    ("pipeline.qdrant_projection_v4_attempts", "deny_qdrant_v4_attempt_truncate"),
    ("pipeline.qdrant_projection_v4_receipts", "deny_qdrant_v4_receipt_rewrite"),
    ("pipeline.qdrant_projection_v4_receipts", "deny_qdrant_v4_receipt_truncate"),
)


def check_readiness_audit(text: str) -> list[str]:
    issues: list[str] = []
    for relation, (policies, _) in NEW_TENANT_POLICIES.items():
        if text.count(f"'{relation}'::regclass") < 2:
            issues.append(f"additional tenant relation not included in owner and RLS catalog audit: {relation}")
        expected_catalog = f"('{relation}', {len(policies)}, '{policies[0]}', " + (
            f"'{policies[1]}')" if len(policies) == 2 else "NULL)"
        )
        if expected_catalog not in text:
            issues.append(f"incorrect per-table policy catalog: {relation}")
    for relation, guard in EXPECTED_GUARDS:
        if f"('{relation}', '{guard}')" not in text:
            issues.append(f"missing original immutable owner trigger catalog entry: {relation}/{guard}")
    required_guards = (
        "rel.relrowsecurity",
        "rel.relforcerowsecurity",
        "policies.policy_count <> expected.expected_policy_count",
        "policies.accepted_count <> expected.expected_policy_count",
        "p.polname = expected.owner_policy",
        "p.polname=expected.insert_policy",
        "p.polcmd = (CASE",
        "p.polcmd='a'",
        "p.polcmd <> '*'",
        "p.polcmd <> '*' OR p.polwithcheck IS NULL OR",
        "p.polcmd <> 'r'",
        "p.polroles = ARRAY[0::oid]",
        "pg_catalog.pg_has_role",
        "has_table_privilege(login.rolname, protected.oid, 'TRUNCATE')",
        "has_table_privilege(login.rolname, protected.oid, 'DELETE')",
        "grant_acl.grantee = 0",
        "owner.rolsuper OR owner.rolbypassrls",
        "guard_vector_projection_source_immutable_v1",
        "guard_vector_projection_source_truncate_v1",
        "guard_vector_projection_admission_immutable_v1",
        "guard_operation_artifact_receipt_immutable_v1",
        "guard_operation_artifact_receipt_truncate_v1",
        "checked_tables <> 16",
        "t.tgrelid = required_guard.relation_name::regclass",
        "t.tgname = required_guard.trigger_name",
        "t.tgenabled IN ('O','A')",
        "PG_PROJECTION_FORCE_RLS_ADDITIONAL_SIXTEEN_RELATIONS=READY",
    )
    for required in required_guards:
        if required not in text:
            issues.append(f"missing fail-closed readiness guard: {required}")
    for line in text.splitlines():
        if line.lstrip().startswith("--"):
            continue
        if re.match(r"\s*(?:CREATE|ALTER|DROP|TRUNCATE|GRANT|REVOKE|INSERT|UPDATE|DELETE)\s+", line, re.I):
            issues.append(f"read-only DBA audit contains a persistent mutation: {line.strip()}")
    return issues


class PgProjectionForceRlsReadinessTests(unittest.TestCase):
    def test_audit_enumerates_all_new_tables_and_never_mutates(self) -> None:
        self.assertEqual(len(NEW_TENANT_POLICIES), 16)
        self.assertEqual(check_readiness_audit(AUDIT.read_text(encoding="utf-8")), [])

    def test_missing_tenant_table_or_force_guard_fails_closed(self) -> None:
        audit = AUDIT.read_text(encoding="utf-8")
        for relation in NEW_TENANT_POLICIES:
            tampered = audit.replace(f"'{relation}'::regclass", "'pipeline.execution_runs'::regclass")
            self.assertTrue(check_readiness_audit(tampered), relation)
        for relation, (policies, _) in NEW_TENANT_POLICIES.items():
            tampered = audit.replace(f"'{policies[0]}'", "'foreign_permissive_policy'")
            self.assertTrue(check_readiness_audit(tampered), relation)
        for relation, guard in EXPECTED_GUARDS:
            tampered = audit.replace(f"('{relation}', '{guard}')", "('pipeline.execution_runs', 'foreign_trigger')")
            self.assertTrue(check_readiness_audit(tampered), guard)
        self.assertTrue(check_readiness_audit(audit.replace("rel.relforcerowsecurity", "TRUE")))
        self.assertTrue(check_readiness_audit(audit + "\nDELETE FROM pipeline.operation_artifact_receipts_v1;\n"))

    def test_all_source_migrations_only_enable_rls(self) -> None:
        for relation, (policies, migration_number) in NEW_TENANT_POLICIES.items():
            migration_path = next(MIGRATIONS.glob(f"20261125_{migration_number}_*.sql"))
            migration = migration_path.read_text(encoding="utf-8")
            self.assertIn(f"ALTER TABLE {relation} ENABLE ROW LEVEL SECURITY;", migration)
            self.assertNotIn(f"ALTER TABLE {relation} FORCE ROW LEVEL SECURITY;", migration)
            for policy in policies:
                self.assertIn(policy, migration)
        migration = SQL_0021.read_text(encoding="utf-8")
        self.assertIn("REVOKE ALL ON FUNCTION pipeline.operation_artifact_receipt_matches_v1", migration)
        self.assertIn("REVOKE ALL ON FUNCTION pipeline.consume_pg_vector_projection_owner_v1", migration)


if __name__ == "__main__":
    unittest.main()
