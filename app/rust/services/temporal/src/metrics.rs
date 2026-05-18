use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Instant;

use infrastructure::adapters::hyper_adapter::{
    http1, service_fn, Bytes, Full, Incoming, Method, Request, Response, StatusCode, TokioIo,
};
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder,
};
use tokio::net::TcpListener;

pub struct Metrics {
    registry: Registry,
    pub workflow_starts_total: IntCounterVec,
    pub workflow_completions_total: IntCounterVec,
    #[allow(dead_code)]
    pub workflow_failures_total: IntCounterVec,
    pub activity_attempts_total: IntCounterVec,
    pub activity_failures_total: IntCounterVec,
    pub step_execution_reused_total: IntCounterVec,
    pub activity_duration_seconds: HistogramVec,
    pub support_bundle_events_total: IntCounterVec,
    pub editorial_provider_events_total: IntCounterVec,
    pub publish_build_scope_total: IntCounterVec,
    pub render_validation_failures_total: IntCounterVec,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

pub fn global() -> &'static Metrics {
    METRICS.get_or_init(|| {
        let registry =
            Registry::new_custom(Some("alegria".to_string()), None).expect("metrics registry");
        let workflow_starts_total = IntCounterVec::new(
            Opts::new("workflow_starts_total", "Workflow starts"),
            &["workflow_type"],
        )
        .expect("workflow_starts_total");
        let workflow_completions_total = IntCounterVec::new(
            Opts::new("workflow_completions_total", "Workflow completions"),
            &["workflow_type"],
        )
        .expect("workflow_completions_total");
        let workflow_failures_total = IntCounterVec::new(
            Opts::new("workflow_failures_total", "Workflow failures"),
            &["workflow_type"],
        )
        .expect("workflow_failures_total");
        let activity_attempts_total = IntCounterVec::new(
            Opts::new("activity_attempts_total", "Activity attempts"),
            &["step_name"],
        )
        .expect("activity_attempts_total");
        let activity_failures_total = IntCounterVec::new(
            Opts::new("activity_failures_total", "Activity failures"),
            &["step_name", "error_class"],
        )
        .expect("activity_failures_total");
        let step_execution_reused_total = IntCounterVec::new(
            Opts::new("step_execution_reused_total", "Execution ledger reuse hits"),
            &["step_name"],
        )
        .expect("step_execution_reused_total");
        let activity_duration_seconds = HistogramVec::new(
            HistogramOpts::new("activity_duration_seconds", "Activity duration seconds"),
            &["step_name"],
        )
        .expect("activity_duration_seconds");
        let support_bundle_events_total = IntCounterVec::new(
            Opts::new("support_bundle_events_total", "Support bundle events"),
            &["outcome"],
        )
        .expect("support_bundle_events_total");
        let editorial_provider_events_total = IntCounterVec::new(
            Opts::new(
                "editorial_provider_events_total",
                "Editorial provider events",
            ),
            &["provider_key", "outcome"],
        )
        .expect("editorial_provider_events_total");
        let publish_build_scope_total = IntCounterVec::new(
            Opts::new(
                "publish_build_scope_total",
                "Publish materialization build scopes",
            ),
            &["build_scope", "outcome"],
        )
        .expect("publish_build_scope_total");
        let render_validation_failures_total = IntCounterVec::new(
            Opts::new(
                "render_validation_failures_total",
                "Render validation failures",
            ),
            &["reason"],
        )
        .expect("render_validation_failures_total");

        for collector in [
            Box::new(workflow_starts_total.clone()) as Box<dyn prometheus::core::Collector>,
            Box::new(workflow_completions_total.clone()),
            Box::new(workflow_failures_total.clone()),
            Box::new(activity_attempts_total.clone()),
            Box::new(activity_failures_total.clone()),
            Box::new(step_execution_reused_total.clone()),
            Box::new(activity_duration_seconds.clone()),
            Box::new(support_bundle_events_total.clone()),
            Box::new(editorial_provider_events_total.clone()),
            Box::new(publish_build_scope_total.clone()),
            Box::new(render_validation_failures_total.clone()),
        ] {
            registry.register(collector).expect("register collector");
        }

        // Pre-initialize required series so the metrics contract is visible even
        // before the first specific event path is exercised.
        for workflow_type in [
            "ContentGenerationWorkflow",
            "FreshnessCheckWorkflow",
            "SeoSiteBuildWorkflow",
            "TestHitlWorkflow",
        ] {
            let _ = workflow_starts_total.with_label_values(&[workflow_type]);
            let _ = workflow_completions_total.with_label_values(&[workflow_type]);
            let _ = workflow_failures_total.with_label_values(&[workflow_type]);
        }
        for step_name in [
            "generate_content",
            "validate_blocks",
            "finalize_run",
            "check_data_freshness",
            "bootstrap",
        ] {
            let _ = activity_attempts_total.with_label_values(&[step_name]);
            let _ = step_execution_reused_total.with_label_values(&[step_name]);
            let _ = activity_duration_seconds.with_label_values(&[step_name]);
        }
        for (step_name, error_class) in [
            ("generate_content", "bootstrap"),
            ("validate_blocks", "bootstrap"),
            ("finalize_run", "bootstrap"),
            ("load_verified_support_bundle", "bootstrap"),
            ("editorial_draft_generate", "bootstrap"),
            ("publish_materialize", "bootstrap"),
            ("render_preview_validate", "bootstrap"),
        ] {
            let _ = activity_failures_total.with_label_values(&[step_name, error_class]);
        }
        for outcome in ["loaded", "empty", "failed"] {
            let _ = support_bundle_events_total.with_label_values(&[outcome]);
        }
        for (provider_key, outcome) in [
            ("openai", "selected"),
            ("anthropic", "selected"),
            ("gemini", "selected"),
            ("local_compatible", "selected"),
            ("deterministic_fixture", "selected"),
            ("deterministic_fixture", "fallback"),
            ("unknown", "failed"),
        ] {
            let _ = editorial_provider_events_total.with_label_values(&[provider_key, outcome]);
        }
        for (build_scope, outcome) in [
            ("page", "built"),
            ("site", "built"),
            ("page", "failed"),
            ("site", "failed"),
        ] {
            let _ = publish_build_scope_total.with_label_values(&[build_scope, outcome]);
        }
        for reason in [
            "missing_html",
            "missing_breadcrumbs",
            "missing_schema_markup",
            "missing_rendered_links",
            "blocked",
        ] {
            let _ = render_validation_failures_total.with_label_values(&[reason]);
        }

        Metrics {
            registry,
            workflow_starts_total,
            workflow_completions_total,
            workflow_failures_total,
            activity_attempts_total,
            activity_failures_total,
            step_execution_reused_total,
            activity_duration_seconds,
            support_bundle_events_total,
            editorial_provider_events_total,
            publish_build_scope_total,
            render_validation_failures_total,
        }
    })
}

pub struct ActivityTimer {
    step_name: String,
    started: Instant,
}

impl ActivityTimer {
    pub fn start(step_name: &str) -> Self {
        global()
            .activity_attempts_total
            .with_label_values(&[step_name])
            .inc();
        Self {
            step_name: step_name.to_string(),
            started: Instant::now(),
        }
    }

    pub fn record_success(self) {
        global()
            .activity_duration_seconds
            .with_label_values(&[&self.step_name])
            .observe(self.started.elapsed().as_secs_f64());
    }

    pub fn record_failure(self, error_class: &str) {
        global()
            .activity_duration_seconds
            .with_label_values(&[&self.step_name])
            .observe(self.started.elapsed().as_secs_f64());
        global()
            .activity_failures_total
            .with_label_values(&[&self.step_name, error_class])
            .inc();
    }
}

async fn serve_metrics(_req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let encoder = TextEncoder::new();
    let metric_families = global().registry.gather();
    let mut buffer = Vec::new();
    let response = if encoder.encode(&metric_families, &mut buffer).is_ok() {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", encoder.format_type())
            .body(Full::new(Bytes::from(buffer)))
            .expect("metrics response")
    } else {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from_static(b"metrics encode error")))
            .expect("metrics error response")
    };
    Ok(response)
}

async fn route(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/metrics") => serve_metrics(req).await,
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from_static(b"not found")))
            .expect("not found response")),
    }
}

pub async fn start_metrics_server(port: u16) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let io = TokioIo::new(stream);
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(io, service_fn(route))
                    .await;
            });
        }
    });
    Ok(())
}
