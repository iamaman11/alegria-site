"""Fail-closed, offline checks for #94's additional three-tenant-table audit.

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

NEW_TENANT_RELATIONS = (
    "pipeline.vector_projection_sources_v1",
    "pipeline.pg_vector_projection_admissions_v1",
    "pipeline.operation_artifact_receipts_v1",
)


def check_readiness_audit(text: str) -> list[str]:
    issues: list[str] = []
    for relation in NEW_TENANT_RELATIONS:
        if text.count(f"'{relation}'::regclass") < 2:
            issues.append(f"additional tenant relation not included in owner and RLS catalog audit: {relation}")
    required_guards = (
        "rel.relrowsecurity",
        "rel.relforcerowsecurity",
        "policies.policy_count <> expected.expected_policy_count",
        "policies.accepted_count <> expected.expected_policy_count",
        "pg_catalog.pg_has_role",
        "has_table_privilege(login.rolname, protected.oid, 'TRUNCATE')",
        "grant_acl.grantee = 0",
        "owner.rolsuper OR owner.rolbypassrls",
        "guard_vector_projection_source_immutable_v1",
        "guard_vector_projection_source_truncate_v1",
        "guard_vector_projection_admission_immutable_v1",
        "guard_operation_artifact_receipt_immutable_v1",
        "guard_operation_artifact_receipt_truncate_v1",
        "PG_PROJECTION_FORCE_RLS_ADDITIONAL_THREE_RELATIONS=READY",
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
        self.assertEqual(check_readiness_audit(AUDIT.read_text(encoding="utf-8")), [])

    def test_missing_tenant_table_or_force_guard_fails_closed(self) -> None:
        audit = AUDIT.read_text(encoding="utf-8")
        for relation in NEW_TENANT_RELATIONS:
            tampered = audit.replace(f"'{relation}'::regclass", "'pipeline.execution_runs'::regclass")
            self.assertTrue(check_readiness_audit(tampered), relation)
        self.assertTrue(check_readiness_audit(audit.replace("rel.relforcerowsecurity", "TRUE")))
        self.assertTrue(check_readiness_audit(audit + "\nDELETE FROM pipeline.operation_artifact_receipts_v1;\n"))

    def test_sql_0021_never_implies_force_rls_was_deployed(self) -> None:
        migration = SQL_0021.read_text(encoding="utf-8")
        for relation in NEW_TENANT_RELATIONS:
            self.assertIn(f"ALTER TABLE {relation} ENABLE ROW LEVEL SECURITY;", migration)
            self.assertNotIn(f"ALTER TABLE {relation} FORCE ROW LEVEL SECURITY;", migration)
        self.assertIn("REVOKE ALL ON FUNCTION pipeline.operation_artifact_receipt_matches_v1", migration)
        self.assertIn("REVOKE ALL ON FUNCTION pipeline.consume_pg_vector_projection_owner_v1", migration)


if __name__ == "__main__":
    unittest.main()
