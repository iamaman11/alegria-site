use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use contracts::generated::alegria::temporal::v1::{FactExtractionInputPayload, ValidationInputPayload};
use infrastructure::adapters::sqlx_reconcile_adapter::{
    reconcile_target_system_default, ReconcileOptionsRecord,
};
use prost::Message;
use primitives::fact_verifier_json::{verify_fact_json, verify_numeric_rule_json};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(name = "cli_tools")]
#[command(about = "Alegria Rust CLI tools", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    CheckRustMigrationContract {
        #[arg(long, default_value = ".")]
        root: String,
        #[arg(long, default_value = "automation/reports/rust_migration_contract.rust.json")]
        report_json: String,
        #[arg(long, default_value_t = false)]
        strict: bool,
    },
    CheckFactVerifierParity {
        #[arg(long, default_value = ".")]
        root: String,
        #[arg(long, default_value = "automation/reports/fact_verifier_parity.rust.json")]
        report_json: String,
    },
    ComputeContentHash {
        #[arg(long)]
        text: String,
    },
    ComputeBytesHash {
        #[arg(long)]
        hex: String,
    },
    ComputeStableRuleId {
        #[arg(long, num_args = 1..)]
        parts: Vec<String>,
    },
    EncodeFactInput {
        #[arg(long)]
        sections_json: String,
    },
    EncodeValidationInput {
        #[arg(long)]
        required_links_json: String,
        #[arg(long)]
        required_keys_json: String,
        #[arg(long)]
        used_rule_keys_json: String,
        #[arg(long)]
        used_fact_keys_json: String,
        #[arg(long)]
        url_norm: String,
    },
    ReconcileTargetSystem {
        #[arg(long)]
        target_system: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = 10)]
        max_retry_count: i32,
        #[arg(long, default_value_t = 500)]
        batch_limit: i64,
        #[arg(long, default_value_t = 0)]
        requeue_base_delay_sec: i64,
        #[arg(long, default_value_t = 15)]
        requeue_jitter_sec: i64,
    },
}

#[derive(Debug)]
struct Finding {
    level: &'static str,
    code: &'static str,
    message: String,
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("read file failed: {}", path.display()))
}

fn write_report(root: &Path, report_path: &str, payload: &Value) -> Result<PathBuf> {
    let out = root.join(report_path);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create report dir failed: {}", parent.display()))?;
    }
    fs::write(&out, serde_json::to_vec_pretty(payload)?)
        .with_context(|| format!("write report failed: {}", out.display()))?;
    Ok(out)
}

fn print_hex(bytes: &[u8]) {
    println!("{}", hex::encode(bytes));
}

fn check_rust_migration_contract(root: &Path, report_json: &str, strict: bool) -> Result<i32> {
    let mut findings: Vec<Finding> = Vec::new();

    // ---------------------------------------------------------------------
    // M001: Required Rust-first files must exist.
    // Legacy Python paths are intentionally NOT required anymore (R9 archive).
    // ---------------------------------------------------------------------
    let required_files = [
        "app/rust/crates/primitives/src/facts_extractor.rs",
        "app/rust/crates/primitives/src/block_validator.rs",
        "app/rust/crates/primitives/src/fact_verifier.rs",
        "app/rust/crates/primitives/src/writer.rs",
        "app/rust/crates/primitives/src/pdf_parser.rs",
        "app/rust/crates/use_cases/src/pipeline_runtime.rs",
        "app/rust/crates/infrastructure/src/adapters/sqlx_pipeline_runtime_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/proto_runtime_payload_store.rs",
        "app/rust/services/temporal/src/activities/mod.rs",
        "app/rust/services/temporal/src/workflows/mod.rs",
        "app/rust/crates/infrastructure/src/adapters/sqlx_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/neo4rs_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/qdrant_client_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/tonic_adapter.rs",
        "app/rust/services/analytics_svc/src/main.rs",
        "app/analytics_lab/proto/analytics.proto",
    ];

    for rel in required_files {
        let p = root.join(rel);
        if !p.exists() {
            findings.push(Finding {
                level: "error",
                code: "M001",
                message: format!("missing required file: {}", p.display()),
            });
        }
    }

    // ---------------------------------------------------------------------
    // R101: Enforce Rust Temporal activity coverage (MERGED_MODE target chain).
    // ---------------------------------------------------------------------
    let temporal_activities = root.join("app/rust/services/temporal/src/activities/mod.rs");
    if temporal_activities.exists() {
        let text = read_text(&temporal_activities)?;
        for fn_name in [
            "pub async fn extract_facts(",
            "pub async fn verify_rules(",
            "pub async fn persist_and_emit(",
            "pub async fn generate_content(",
            "pub async fn validate_blocks(",
            "pub async fn finalize_run(",
        ] {
            if !text.contains(fn_name) {
                findings.push(Finding {
                    level: "warn",
                    code: "R101",
                    message: format!("missing temporal activity in Rust worker: {fn_name}"),
                });
            }
        }
    }

    // ---------------------------------------------------------------------
    // R102: outbox dedup protection must be present in schema and use-case SQL.
    // ---------------------------------------------------------------------
    let schema_sql = root.join("app/db/schema.sql");
    if schema_sql.exists() {
        let schema = read_text(&schema_sql)?;
        if !schema.contains("idx_sync_outbox_dedup") {
            findings.push(Finding {
                level: "warn",
                code: "R102",
                message: "schema.sql missing unique dedup index idx_sync_outbox_dedup".to_string(),
            });
        }
    }

    let pipeline_storage_files = [
        root.join("app/rust/crates/infrastructure/src/adapters/proto_runtime_payload_store.rs"),
        root.join("app/rust/crates/infrastructure/src/adapters/sqlx_runtime_outbox_adapter.rs"),
        root.join("app/rust/crates/infrastructure/src/adapters/sqlx_source_projection_adapter.rs"),
        root.join("app/rust/crates/infrastructure/src/adapters/sqlx_step_ledger_adapter.rs"),
    ];
    if pipeline_storage_files.iter().any(|path| path.exists()) {
        let mut ptxt = String::new();
        for path in pipeline_storage_files {
            if path.exists() {
                ptxt.push_str(&read_text(&path)?);
                ptxt.push('\n');
            }
        }
        if !ptxt.contains("ON CONFLICT") {
            findings.push(Finding {
                level: "warn",
                code: "R102",
                message: "pipeline_storage outbox emit path has no ON CONFLICT dedup guard".to_string(),
            });
        }
    }

    // ---------------------------------------------------------------------
    // R103: Python runtime files under app/ are forbidden.
    // ---------------------------------------------------------------------
    let app_root = root.join("app");
    if app_root.exists() {
        fn scan_py(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
            for ent in fs::read_dir(dir).with_context(|| format!("read_dir failed: {}", dir.display()))? {
                let ent = ent?;
                let path = ent.path();
                if path.is_dir() {
                    let name = ent.file_name();
                    let name = name.to_string_lossy();
                    if name == "__pycache__" || name == ".venv" || name == "target" {
                        continue;
                    }
                    scan_py(&path, out)?;
                } else if path.extension().and_then(|e| e.to_str()) == Some("py") {
                    out.push(path);
                }
            }
            Ok(())
        }

        let mut py_files = Vec::new();
        scan_py(&app_root, &mut py_files)?;
        for p in py_files {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().to_string();
            findings.push(Finding {
                level: "warn",
                code: "R103",
                message: format!("python runtime file present in app (expected 0): {rel}"),
            });
        }
    }

    let mut status = "ok";
    if findings.iter().any(|f| f.level == "error") {
        status = "failed";
    } else if strict && findings.iter().any(|f| f.code.starts_with('R')) {
        status = "failed";
    }

    let report = json!({
        "status": status,
        "strict": strict,
        "findings": findings.iter().map(|f| json!({
            "level": f.level,
            "code": f.code,
            "message": f.message,
        })).collect::<Vec<_>>()
    });
    let out = write_report(root, report_json, &report)?;

    println!("RUST_MIGRATION_CONTRACT: {}", status.to_uppercase());
    println!("report: {}", out.display());
    for f in &findings {
        println!("[{}] {}: {}", f.level, f.code, f.message);
    }

    Ok(if status == "ok" { 0 } else { 1 })
}

fn check_fact_verifier_parity(root: &Path, report_json: &str) -> Result<i32> {
    let mut findings: Vec<Finding> = Vec::new();

    let registry = json!({
        "govA": {"source_type": "government", "trust_level": 5},
        "vfsA": {"source_type": "vfs", "trust_level": 4},
        "ag1": {"source_type": "niche_agency", "trust_level": 2},
        "ag2": {"source_type": "niche_agency", "trust_level": 2},
        "ed1": {"source_type": "editorial", "trust_level": 3},
    });

    // Case 1: empty -> hitl_required
    let r1 = verify_fact_json("consular_fee", "[]", &registry.to_string());
    let r1v: Value = serde_json::from_str(&r1)?;
    if r1v.get("resolution").and_then(Value::as_str) != Some("hitl_required") {
        findings.push(Finding {
            level: "error",
            code: "FVP001",
            message: "empty candidates must return hitl_required".to_string(),
        });
    }

    // Case 2: gov/vfs authoritative wins by confidence
    let c2 = json!([
        {"source_key":"ag1","fact_value":35,"extraction_confidence":0.95},
        {"source_key":"govA","fact_value":90,"extraction_confidence":0.60},
        {"source_key":"vfsA","fact_value":80,"extraction_confidence":0.90}
    ]);
    let r2: Value = serde_json::from_str(&verify_fact_json("consular_fee", &c2.to_string(), &registry.to_string()))?;
    if r2.get("resolution").and_then(Value::as_str) != Some("gov_wins") {
        findings.push(Finding {
            level: "error",
            code: "FVP002",
            message: "authoritative case must return gov_wins".to_string(),
        });
    }
    if r2.get("value").and_then(Value::as_i64) != Some(80) {
        findings.push(Finding {
            level: "error",
            code: "FVP003",
            message: "authoritative winner must pick highest confidence value=80".to_string(),
        });
    }

    // Case 3: consensus >= 60%
    let c3 = json!([
        {"source_key":"ag1","fact_value":15,"extraction_confidence":0.70},
        {"source_key":"ag2","fact_value":15,"extraction_confidence":0.72},
        {"source_key":"ed1","fact_value":20,"extraction_confidence":0.65},
        {"source_key":"ag2","fact_value":15,"extraction_confidence":0.60},
        {"source_key":"ag1","fact_value":20,"extraction_confidence":0.60}
    ]);
    let r3: Value = serde_json::from_str(&verify_fact_json("processing_days", &c3.to_string(), &registry.to_string()))?;
    if r3.get("resolution").and_then(Value::as_str) != Some("consensus") {
        findings.push(Finding {
            level: "error",
            code: "FVP004",
            message: "consensus case must return consensus".to_string(),
        });
    }
    if r3.get("value").and_then(Value::as_i64) != Some(15) {
        findings.push(Finding {
            level: "error",
            code: "FVP005",
            message: "consensus winner must be value=15".to_string(),
        });
    }

    // Case 4: conflict -> hitl_required with diagnostics
    let c4 = json!([
        {"source_key":"ag1","fact_value":7,"extraction_confidence":0.80},
        {"source_key":"ag2","fact_value":9,"extraction_confidence":0.81},
        {"source_key":"ed1","fact_value":11,"extraction_confidence":0.82}
    ]);
    let r4: Value = serde_json::from_str(&verify_fact_json("processing_days", &c4.to_string(), &registry.to_string()))?;
    if r4.get("resolution").and_then(Value::as_str) != Some("hitl_required") {
        findings.push(Finding {
            level: "error",
            code: "FVP006",
            message: "conflict case must return hitl_required".to_string(),
        });
    }

    // Numeric range merge
    let c5 = json!([
        {"params":{"amount":35.0}},
        {"params":{"amount":90.0}},
        {"params":{"amount":60.0}}
    ]);
    let r5: Value = serde_json::from_str(&verify_numeric_rule_json("consular_fee", &c5.to_string(), "amount"))?;
    if r5.get("resolution").and_then(Value::as_str) != Some("range_merged") {
        findings.push(Finding {
            level: "error",
            code: "FVP007",
            message: "numeric case must return range_merged".to_string(),
        });
    }
    let range = r5.get("range").cloned().unwrap_or(json!({}));
    if range.get("min").and_then(Value::as_f64) != Some(35.0)
        || range.get("max").and_then(Value::as_f64) != Some(90.0)
        || range.get("count").and_then(Value::as_u64) != Some(3)
    {
        findings.push(Finding {
            level: "error",
            code: "FVP008",
            message: "numeric range expected min=35 max=90 count=3".to_string(),
        });
    }

    let status = if findings.iter().any(|f| f.level == "error") {
        "failed"
    } else {
        "ok"
    };

    let report = json!({
        "status": status,
        "cases_total": 5,
        "findings": findings.iter().map(|f| json!({
            "level": f.level,
            "code": f.code,
            "message": f.message,
        })).collect::<Vec<_>>()
    });
    let out = write_report(root, report_json, &report)?;

    println!("FACT_VERIFIER_PARITY: {}", status.to_uppercase());
    println!("report: {}", out.display());
    for f in &findings {
        println!("[{}] {}: {}", f.level, f.code, f.message);
    }

    Ok(if status == "ok" { 0 } else { 1 })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let code = match cli.command {
        Command::CheckRustMigrationContract {
            root,
            report_json,
            strict,
        } => check_rust_migration_contract(Path::new(&root), &report_json, strict)?,
        Command::CheckFactVerifierParity { root, report_json } => {
            check_fact_verifier_parity(Path::new(&root), &report_json)?
        }
        Command::ComputeContentHash { text } => {
            println!("{}", primitives::hash::content_hash_v1(&text));
            0
        }
        Command::ComputeBytesHash { hex } => {
            let bytes = hex::decode(&hex).context("invalid hex for ComputeBytesHash")?;
            println!("{}", primitives::hash::blake3_hex(&bytes));
            0
        }
        Command::ComputeStableRuleId { parts } => {
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            println!("{}", primitives::stable_id::stable_rule_instance_id(&refs));
            0
        }
        Command::EncodeFactInput { sections_json } => {
            let bytes = FactExtractionInputPayload { sections_json }.encode_to_vec();
            print_hex(&bytes);
            0
        }
        Command::EncodeValidationInput {
            required_links_json,
            required_keys_json,
            used_rule_keys_json,
            used_fact_keys_json,
            url_norm,
        } => {
            let bytes = ValidationInputPayload {
                meta: None,
                required_links_json_utf8: required_links_json.into_bytes(),
                required_keys_json_utf8: required_keys_json.into_bytes(),
                used_rule_keys_json_utf8: used_rule_keys_json.into_bytes(),
                used_fact_keys_json_utf8: used_fact_keys_json.into_bytes(),
                url_norm,
            }
            .encode_to_vec();
            print_hex(&bytes);
            0
        }
        Command::ReconcileTargetSystem {
            target_system,
            dry_run,
            max_retry_count,
            batch_limit,
            requeue_base_delay_sec,
            requeue_jitter_sec,
        } => {
            let report = reconcile_target_system_default(
                &target_system,
                &ReconcileOptionsRecord {
                    max_retry_count,
                    batch_limit,
                    dry_run,
                    requeue_base_delay_sec,
                    requeue_jitter_sec,
                },
            )
            .await?;
            println!(
                "{}|{}|{}|{}|{}|{}",
                report.target_system,
                report.dry_run,
                report.stale_candidates,
                report.failed_candidates,
                report.reset_stale_processing,
                report.requeued_failed
            );
            0
        }
    };

    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code)
    }
}
