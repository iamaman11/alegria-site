use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{HeaderValue, StatusCode},
    response::Response,
    routing::any,
    Json, Router,
};
use serde_json::Value;
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

#[derive(Debug)]
pub struct StubServerHandle {
    pub base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl Drop for StubServerHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

#[derive(Clone)]
struct StubState {
    payload: Arc<Value>,
}

#[derive(Clone)]
struct TextStubState {
    body: Arc<String>,
    content_type: &'static str,
}

#[derive(Clone)]
struct SequenceStubState {
    payloads: Arc<Vec<Value>>,
    next_index: Arc<AtomicUsize>,
}

async fn serve_payload(State(state): State<StubState>) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json((*state.payload).clone()))
}

async fn serve_payload_sequence(
    State(state): State<SequenceStubState>,
) -> (StatusCode, Json<Value>) {
    let idx = state.next_index.fetch_add(1, Ordering::SeqCst);
    let payload = state
        .payloads
        .get(idx)
        .cloned()
        .or_else(|| state.payloads.last().cloned())
        .unwrap_or_else(|| serde_json::json!({}));
    (StatusCode::OK, Json(payload))
}

async fn serve_text(State(state): State<TextStubState>) -> Response {
    let mut response = Response::new(axum::body::Body::from((*state.body).clone()));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static(state.content_type),
    );
    response
}

pub async fn spawn_json_stub(path: &str, payload: Value) -> Result<StubServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind stub listener failed")?;
    let addr: SocketAddr = listener
        .local_addr()
        .context("read stub listener addr failed")?;
    let state = StubState {
        payload: Arc::new(payload),
    };
    let app = Router::new()
        .route(path, any(serve_payload))
        .with_state(state);
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = rx.await;
        });
        let _ = server.await;
    });
    Ok(StubServerHandle {
        base_url: format!("http://{}", addr),
        shutdown: Some(tx),
        task,
    })
}

pub async fn spawn_json_sequence_stub(
    path: &str,
    payloads: Vec<Value>,
) -> Result<StubServerHandle> {
    if payloads.is_empty() {
        anyhow::bail!("sequence stub requires at least one payload");
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind sequence stub listener failed")?;
    let addr: SocketAddr = listener
        .local_addr()
        .context("read sequence stub listener addr failed")?;
    let state = SequenceStubState {
        payloads: Arc::new(payloads),
        next_index: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new()
        .route(path, any(serve_payload_sequence))
        .with_state(state);
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = rx.await;
        });
        let _ = server.await;
    });
    Ok(StubServerHandle {
        base_url: format!("http://{}", addr),
        shutdown: Some(tx),
        task,
    })
}

pub async fn spawn_text_stub(
    path: &str,
    body: impl Into<String>,
    content_type: &'static str,
) -> Result<StubServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind text stub listener failed")?;
    let addr: SocketAddr = listener
        .local_addr()
        .context("read text stub listener addr failed")?;
    let state = TextStubState {
        body: Arc::new(body.into()),
        content_type,
    };
    let app = Router::new().route(path, any(serve_text)).with_state(state);
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = rx.await;
        });
        let _ = server.await;
    });
    Ok(StubServerHandle {
        base_url: format!("http://{}", addr),
        shutdown: Some(tx),
        task,
    })
}
