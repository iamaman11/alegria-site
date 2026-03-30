use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use infrastructure::adapters::temporalio_sdk_adapter::{
    connect_client, RawValue, UntypedSignal, UntypedWorkflow, WorkflowGetResultOptions,
    WorkflowSignalOptions, WorkflowStartOptions,
};
use std::time::Duration;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "temporal_starter")]
#[command(about = "Temporal starter CLI for Alegria workflows")]
struct Cli {
    /// Temporal endpoint, example: http://localhost:7233
    #[arg(long, default_value = "http://localhost:7233")]
    temporal_url: String,
    /// Temporal namespace
    #[arg(long, default_value = "default")]
    namespace: String,
    /// Task queue (must match worker queue)
    #[arg(long, default_value = "alegria-pipeline")]
    task_queue: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Connectivity check: open client and print namespace/task queue
    Ping,
    /// Start workflow execution
    Start {
        #[arg(long, value_enum)]
        workflow: WorkflowKind,
        /// Optional workflow id; if omitted a deterministic prefix + UUID is used
        #[arg(long)]
        workflow_id: Option<String>,
    },
    /// End-to-end test workflow with HITL pause/resume
    DemoHitl {
        /// Optional workflow id; if omitted generated
        #[arg(long)]
        workflow_id: Option<String>,
        /// Delay before sending resume signal
        #[arg(long, default_value_t = 700)]
        wait_before_resume_ms: u64,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum WorkflowKind {
    FactExtraction,
    ContentGeneration,
    FreshnessCheck,
    TestHitl,
}

impl WorkflowKind {
    fn workflow_type(self) -> &'static str {
        match self {
            WorkflowKind::FactExtraction => "FactExtractionWorkflow",
            WorkflowKind::ContentGeneration => "ContentGenerationWorkflow",
            WorkflowKind::FreshnessCheck => "FreshnessCheckWorkflow",
            WorkflowKind::TestHitl => "TestHitlWorkflow",
        }
    }

    fn id_prefix(self) -> &'static str {
        match self {
            WorkflowKind::FactExtraction => "fact-extract",
            WorkflowKind::ContentGeneration => "content-gen",
            WorkflowKind::FreshnessCheck => "freshness-check",
            WorkflowKind::TestHitl => "test-hitl",
        }
    }

    fn requires_uuid_run_id(self) -> bool {
        matches!(self, WorkflowKind::FactExtraction | WorkflowKind::ContentGeneration)
    }
}

fn empty_payload() -> RawValue {
    RawValue::default()
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let identity = format!("alegria-temporal-starter@{}", std::process::id());
    let client = connect_client(&cli.temporal_url, identity, &cli.namespace)
        .await
        .context("failed to connect to Temporal")?;

    match cli.command {
        Command::Ping => {
            println!(
                "ok temporal_url={} namespace={} task_queue={}",
                cli.temporal_url, cli.namespace, cli.task_queue
            );
        }
        Command::Start {
            workflow,
            workflow_id,
        } => {
            let wf_id = match workflow_id {
                Some(v) => v,
                None if workflow.requires_uuid_run_id() => {
                    // For fact/content workflows workflow_id is treated as run_id in DB layer.
                    // It must be a UUID string to avoid non-deterministic retry loops.
                    Uuid::new_v4().to_string()
                }
                None => format!("{}-{}", workflow.id_prefix(), Uuid::new_v4()),
            };
            let options = WorkflowStartOptions::new(cli.task_queue, wf_id.clone()).build();
            let handle = client
                .start_workflow(
                    UntypedWorkflow::new(workflow.workflow_type()),
                    empty_payload(),
                    options,
                )
                .await
                .with_context(|| format!("failed to start {}", workflow.workflow_type()))?;

            println!(
                "started workflow_type={} workflow_id={} run_id={:?}",
                workflow.workflow_type(),
                handle.info().workflow_id,
                handle.info().run_id
            );
        }
        Command::DemoHitl {
            workflow_id,
            wait_before_resume_ms,
        } => {
            let wf_id = workflow_id
                .unwrap_or_else(|| format!("test-hitl-{}", Uuid::new_v4()));
            let options = WorkflowStartOptions::new(cli.task_queue, wf_id.clone()).build();
            let handle = client
                .start_workflow(UntypedWorkflow::new("TestHitlWorkflow"), empty_payload(), options)
                .await
                .context("failed to start TestHitlWorkflow")?;

            tokio::time::sleep(Duration::from_millis(wait_before_resume_ms)).await;
            handle
                .signal(
                    UntypedSignal::<UntypedWorkflow>::new("resume"),
                    empty_payload(),
                    WorkflowSignalOptions::default(),
                )
                .await
                .context("failed to send resume signal")?;

            let result_raw = handle
                .get_result(WorkflowGetResultOptions::default())
                .await
                .context("failed to get workflow result")?;
            println!(
                "demo_hitl_ok workflow_id={} run_id={:?} result_payloads={}",
                handle.info().workflow_id,
                handle.info().run_id,
                result_raw.payloads.len()
            );
        }
    }
    Ok(())
}
