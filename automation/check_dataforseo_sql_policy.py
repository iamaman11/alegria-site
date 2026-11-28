"""Fast fail-closed structural checks for the STUB-only DataForSEO owner DDL.

These guard accidental source relaxations before compile. They do not replace
the physical PostgreSQL role/tenant/history/effect admission E2E.
"""

from pathlib import Path
import re
import unittest


MIGRATION = (
    Path(__file__).resolve().parents[1]
    / "app/db/migrations/20261127_0001_dataforseo_owner_v1_admission.sql"
)
PAID_MIGRATION = (
    Path(__file__).resolve().parents[1]
    / "app/db/migrations/20261128_0001_dataforseo_paid_owner_v2.sql"
)


def validate_policy(sql: str) -> None:
    """Raise for missing immutable ownership, tenant or provider-effect gates."""
    normalized = re.sub(r"--[^\n]*", "", sql).lower()
    compact = re.sub(r"\s+", " ", normalized)

    required = (
        "begin;",
        "commit;",
        "check (provider_mode='stub')",
        "check (parent_workflow_id='alegria-operation-dag-v2-' || run_id::text)",
        "check (child_workflow_id='alegria-owner-operation-v2-' || dispatch_execution_key)",
        "unique (run_id,plan_hash,operation_instance)",
        "unique (child_workflow_run_id,operation_instance)",
        "command_sha256=encode(public.digest(command_bytes,'sha256'),'hex')",
        "a.issued_by<>session_user",
        "a.provider_mode<>'stub'",
        "pipeline.operation_dag_governance_matches_v2(",
        "'operation-dag-business|'",
        "'operation-dag-canonical|'",
        "return 'uncertain';",
        "return 'posted';",
        "return 'confirmed';",
        "return 'started';",
        "actual_cost_usd_micros",
        "p_actual_cost_micros<>0",
        "provider_task_id ~ '^stub-serp-[0-9]{1,8}$'",
    )
    for clause in required:
        if clause not in compact:
            raise AssertionError(f"DataForSEO immutable SQL policy missing: {clause}")
    tables = (
        "dataforseo_owner_admissions_v1",
        "dataforseo_owner_attempts_v1",
        "dataforseo_owner_posted_tasks_v1",
        "dataforseo_owner_receipts_v1",
    )
    for table in tables:
        if f"create table pipeline.{table}" not in compact:
            raise AssertionError(f"DataForSEO table missing: {table}")
        if f"alter table pipeline.{table} enable row level security;" not in compact:
            raise AssertionError(f"DataForSEO RLS missing: {table}")
        if f"on pipeline.{table}" not in compact:
            raise AssertionError(f"DataForSEO trigger/policy missing: {table}")
    functions = (
        "lookup_dataforseo_owner_admission_v1",
        "begin_dataforseo_owner_v1",
        "record_stub_dataforseo_task_v1",
        "lookup_stub_dataforseo_task_v1",
        "confirm_dataforseo_owner_v1",
        "read_confirmed_dataforseo_owner_receipt_v1",
    )
    for func in functions:
        if f"create function pipeline.{func}" not in compact:
            raise AssertionError(f"DataForSEO SQL capability missing: {func}")
        if f"revoke all on function pipeline.{func}" not in compact:
            raise AssertionError(f"DataForSEO public default EXECUTE not revoked: {func}")
    for table in tables[1:]:
        if f"on pipeline.{table} for each row execute function pipeline.deny_dataforseo_owner_ledger_rewrite_v1();" not in compact:
            raise AssertionError(f"DataForSEO append-only ledger missing: {table}")
    if re.search(r"(?m)^\s*grant\s+", normalized):
        raise AssertionError("DataForSEO migration must never grant a live owner permission")


def validate_paid_policy(sql: str) -> None:
    """Review the isolated, opt-in paid V2 source gates without granting them."""
    normalized = re.sub(r"--[^\n]*", "", sql).lower()
    compact = re.sub(r"\s+", " ", normalized)
    for clause in (
        "begin;",
        "commit;",
        "check (provider_mode='paid')",
        "references pipeline.operation_dag_one_step_request_grants_v1(request_id)",
        "g.state<>'start_confirmed'",
        "g.limits->>'currency_code'<>'usd'",
        "g.limits->>'max_charge_minor_units'",
        "pipeline.operation_dag_governance_matches_v2(",
        "pipeline.current_session_tenant_v1()",
        "a.issued_by=session_user",
        "a.command_sha256=encode(public.digest(p_command_bytes,'sha256'),'hex')",
        "dataforseo_paid_owner_attempts_v2",
        "dataforseo_paid_owner_posted_tasks_v2",
        "dataforseo_paid_owner_receipts_v2",
        "return 'uncertain';",
        "return 'posted';",
        "return 'confirmed';",
        "return 'started';",
    ):
        if clause not in compact:
            raise AssertionError(f"Paid DataForSEO V2 security clause missing: {clause}")
    for table in (
        "dataforseo_paid_owner_admissions_v2",
        "dataforseo_paid_owner_attempts_v2",
        "dataforseo_paid_owner_posted_tasks_v2",
        "dataforseo_paid_owner_receipts_v2",
    ):
        if f"create table pipeline.{table}" not in compact:
            raise AssertionError(f"Paid DataForSEO V2 table missing: {table}")
        for gate in ("enable row level security", "force row level security"):
            if f"alter table pipeline.{table} {gate};" not in compact:
                raise AssertionError(f"Paid DataForSEO V2 {gate} missing: {table}")
        if f"revoke all on pipeline.{table}" not in compact:
            # The first table starts a grouped REVOKE with four table names.
            if table != "dataforseo_paid_owner_admissions_v2" or (
                "revoke all on pipeline.dataforseo_paid_owner_admissions_v2," not in compact
            ):
                # Grouped REVOKE is also covered by explicit table name presence above.
                if f"pipeline.{table}" not in compact.split(" from public;", 1)[0]:
                    raise AssertionError(f"Paid DataForSEO V2 table REVOKE missing: {table}")
    for name in (
        "lookup_dataforseo_paid_owner_admission_v2",
        "begin_dataforseo_paid_owner_v2",
        "record_dataforseo_paid_task_v2",
        "read_dataforseo_paid_task_v2",
        "confirm_dataforseo_paid_owner_v2",
        "read_confirmed_dataforseo_paid_owner_receipt_v2",
    ):
        if f"create function pipeline.{name}" not in compact:
            raise AssertionError(f"Paid DataForSEO V2 function missing: {name}")
        if f"revoke all on function pipeline.{name}" not in compact:
            raise AssertionError(f"Paid DataForSEO V2 default EXECUTE not revoked: {name}")
    if re.search(r"(?m)^\s*grant\s+", normalized):
        raise AssertionError("Paid DataForSEO V2 must not grant authority during migration")


class DataForSeoSqlPolicyTests(unittest.TestCase):
    def test_canonical_stub_only_sql_policy(self) -> None:
        validate_policy(MIGRATION.read_text(encoding="utf-8"))

    def test_reject_enabled_live_provider(self) -> None:
        sql = MIGRATION.read_text(encoding="utf-8").replace(
            "CHECK (provider_mode='stub')", "CHECK (provider_mode IN ('stub','live'))"
        )
        with self.assertRaises(AssertionError):
            validate_policy(sql)

    def test_reject_removed_tenant_rls(self) -> None:
        sql = MIGRATION.read_text(encoding="utf-8").replace(
            "ALTER TABLE pipeline.dataforseo_owner_receipts_v1 ENABLE ROW LEVEL SECURITY;",
            "-- incorrectly disabled RLS",
        )
        with self.assertRaises(AssertionError):
            validate_policy(sql)

    def test_reject_unprivileged_worker_grant(self) -> None:
        sql = MIGRATION.read_text(encoding="utf-8") + (
            "\nGRANT INSERT ON pipeline.dataforseo_owner_admissions_v1 TO PUBLIC;\n"
        )
        with self.assertRaises(AssertionError):
            validate_policy(sql)

    def test_paid_v2_isolated_fail_closed_schema(self) -> None:
        validate_paid_policy(PAID_MIGRATION.read_text(encoding="utf-8"))

    def test_paid_v2_refuses_missing_forced_rls(self) -> None:
        sql = PAID_MIGRATION.read_text(encoding="utf-8").replace(
            "ALTER TABLE pipeline.dataforseo_paid_owner_receipts_v2 FORCE ROW LEVEL SECURITY;",
            "-- wrongly disabled FORCE RLS",
        )
        with self.assertRaises(AssertionError):
            validate_paid_policy(sql)

    def test_paid_v2_refuses_accidental_direct_worker_grant(self) -> None:
        sql = PAID_MIGRATION.read_text(encoding="utf-8") + (
            "\nGRANT INSERT ON pipeline.dataforseo_paid_owner_attempts_v2 TO paid_worker;\n"
        )
        with self.assertRaises(AssertionError):
            validate_paid_policy(sql)


if __name__ == "__main__":
    unittest.main()
