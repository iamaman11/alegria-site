//! Opt-in, private-only HTTP transport for the existing signed one-use native
//! Temporal Start. No authoring, SQL grant issuance, tenant selection, proxy,
//! dynamic URL or automatic retry exists at this edge.

use std::{
    env,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{ensure, Context, Result};
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use operation_protocol::SignedOneStepAdmissionV1;
use serde::Serialize;
use tokio::{net::TcpListener, sync::Semaphore};
use uuid::Uuid;

use super::{operator_dag_governance, operator_single_step_start_v2};

#[cfg(test)]
use axum::body::Body;

const ENABLE_ENV: &str = "ALEGRIA_ENABLE_SITE_ARCHITECT_SIGNED_INGRESS_V1";
const PORT_ENV: &str = "SITE_ARCHITECT_SIGNED_INGRESS_V1_PORT";
const INTENT_DIR_ENV: &str = "SITE_ARCHITECT_SIGNED_INGRESS_V1_INTENT_DIR";
const KEY_FILE_ENV: &str = "SITE_ARCHITECT_SIGNED_ADMISSION_V1_KEY_FILE";
const MAX_PROOF_BYTES: usize = 2048;
const NIL_REQUEST_ID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Clone)]
struct IngressState {
    intent_dir: PathBuf,
    start_permit: Arc<Semaphore>,
}

#[derive(Serialize)]
struct IngressReply<'a> {
    schema_revision: u16,
    request_id: &'a str,
    status: &'static str,
    code: &'static str,
}

fn reply(request_id: &str, status: &'static str, code: &'static str, http: StatusCode) -> Response {
    let payload = IngressReply {
        schema_revision: 1,
        request_id,
        status,
        code,
    };
    let mut response = (http, Json(payload)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn checked_port(value: &str) -> Result<u16> {
    let port: u16 = value.parse().context("invalid signed ingress port")?;
    ensure!(
        port >= 1024 && value == port.to_string(),
        "signed ingress requires canonical unprivileged TCP port"
    );
    Ok(port)
}

fn checked_intent_dir(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|component| !matches!(component, Component::ParentDir | Component::CurDir)),
        "signed ingress intent directory must be absolute and unambiguous"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.file_type().is_dir() && metadata.permissions().mode() & 0o077 == 0,
            "signed ingress intent directory must be a private, non-symlink directory"
        );
        for ancestor in path.ancestors() {
            let parent = std::fs::symlink_metadata(ancestor)?;
            ensure!(
                parent.file_type().is_dir()
                    && parent.permissions().mode() & 0o022 == 0
                    && (parent.uid() == 0 || parent.uid() == metadata.uid()),
                "signed ingress intent ancestry is writable or unowned"
            );
        }
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!("signed ingress requires Unix private-file access");
    }
    Ok(path)
}

fn request_id_from_proof(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_PROOF_BYTES {
        return None;
    }
    let proof: SignedOneStepAdmissionV1 = serde_json::from_slice(bytes).ok()?;
    let id = Uuid::parse_str(&proof.request_id).ok()?;
    (!id.is_nil() && id.to_string() == proof.request_id).then_some(proof.request_id)
}

async fn post_admission(State(state): State<IngressState>, request: Request) -> Response {
    if request.uri().query().is_some()
        || request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            != Some("application/json")
    {
        return reply(
            NIL_REQUEST_ID,
            "launch_blocked",
            "untrusted_admission_request",
            StatusCode::BAD_REQUEST,
        );
    }
    // Bound the stream even when Content-Length is missing or misleading.
    let Ok(bytes) = to_bytes(request.into_body(), MAX_PROOF_BYTES).await else {
        return reply(
            NIL_REQUEST_ID,
            "launch_blocked",
            "untrusted_admission_request",
            StatusCode::BAD_REQUEST,
        );
    };
    let Some(request_id) = request_id_from_proof(&bytes) else {
        return reply(
            NIL_REQUEST_ID,
            "launch_blocked",
            "untrusted_admission_request",
            StatusCode::BAD_REQUEST,
        );
    };
    let Ok(_permit) = state.start_permit.clone().try_acquire_owned() else {
        return reply(
            &request_id,
            "launch_blocked",
            "private_operator_busy",
            StatusCode::SERVICE_UNAVAILABLE,
        );
    };
    // The UUID can select only a separately provisioned private file. The
    // native operator validates HMAC/subject *before opening that file* and
    // retains its strict 0600/no-follow directory traversal protections.
    let intent_path = state.intent_dir.join(format!("{request_id}.json"));
    match operator_single_step_start_v2::admit_signed_one_step_bytes(&bytes, &intent_path).await {
        Err(_) => reply(
            &request_id,
            "launch_blocked",
            "signed_remote_admission_not_verified",
            StatusCode::OK,
        ),
        Ok(true) => reply(
            &request_id,
            "request_reserved",
            "one_use_reserved_execution_unverified",
            StatusCode::OK,
        ),
        Ok(false) => reply(
            &request_id,
            "start_outcome_unknown",
            "operator_start_outcome_unconfirmed",
            StatusCode::OK,
        ),
    }
}

/// Never creates a public listener. A local cloudflared/approved private
/// connector must share the loopback network namespace with this ingress.
pub(super) async fn serve_private_ingress() -> Result<()> {
    ensure!(
        env::var(ENABLE_ENV).ok().as_deref() == Some("true"),
        "private signed ingress disabled"
    );
    let port = checked_port(&env::var(PORT_ENV).context("missing private ingress port")?)?;
    let intent_dir =
        checked_intent_dir(&env::var(INTENT_DIR_ENV).context("missing private intent directory")?)?;
    let key_file = env::var(KEY_FILE_ENV).context("missing installed HMAC verification key")?;
    // Fail at startup if key file access is not isolated. Never echo its bytes.
    let _ = operator_dag_governance::read_private_operator_file(
        Path::new(&key_file),
        "signed ingress verification key",
    )?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let state = IngressState {
        intent_dir,
        start_permit: Arc::new(Semaphore::new(1)),
    };
    axum::serve(
        listener,
        Router::new()
            .route("/v1/one-step", post(post_admission))
            .with_state(state),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ingress_accepts_only_canonical_port_and_uuid() {
        assert_eq!(checked_port("9527").unwrap(), 9527);
        for value in ["0", "80", "09527", "-1", "65536", "abc"] {
            assert!(checked_port(value).is_err());
        }
        let id = Uuid::new_v4().to_string();
        let proof = serde_json::json!({
            "schema_revision":1,
            "request_id":id,
            "issuer":"https://example.org",
            "audience":"https://mcp.example.org/mcp",
            "subject":"human",
            "scope":"site_architect.run_one",
            "issued_at_unix_seconds":1900000000,
            "expires_at_unix_seconds":1900000030,
            "nonce":Uuid::new_v4().to_string(),
            "signature_hex":"ab".repeat(32),
        });
        let bytes = serde_json::to_vec(&proof).unwrap();
        assert_eq!(request_id_from_proof(&bytes), Some(id.clone()));
        for rejected in [
            serde_json::json!({"request_id":format!("../{id}")}),
            serde_json::json!({"request_id":id.to_uppercase()}),
            serde_json::json!({"request_id":NIL_REQUEST_ID}),
            serde_json::json!({"request_id":format!("{id}/../../other")}),
        ] {
            assert!(request_id_from_proof(&serde_json::to_vec(&rejected).unwrap()).is_none());
        }
        assert!(request_id_from_proof(&vec![b'a'; MAX_PROOF_BYTES + 1]).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn private_ingress_requires_private_intent_directory_without_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let directory = env::current_dir()
            .unwrap()
            .join(format!(".ci-site-architect-ingress-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            checked_intent_dir(directory.to_str().unwrap()).unwrap(),
            directory
        );

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(checked_intent_dir(directory.to_str().unwrap()).is_err());
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();

        let symlink_path = directory.with_extension("link");
        symlink(&directory, &symlink_path).unwrap();
        assert!(checked_intent_dir(symlink_path.to_str().unwrap()).is_err());
        std::fs::remove_file(&symlink_path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[tokio::test]
    async fn malformed_http_request_cannot_reach_operator() {
        let state = IngressState {
            intent_dir: PathBuf::from("/nonexistent"),
            start_permit: Arc::new(Semaphore::new(1)),
        };
        for (header_value, body) in [
            ("text/plain", Body::from("hello")),
            (
                "application/json",
                Body::from(vec![b'x'; MAX_PROOF_BYTES + 1]),
            ),
            ("application/json", Body::from("{}")),
        ] {
            let request = Request::builder()
                .method("POST")
                .uri("/v1/one-step")
                .header(header::CONTENT_TYPE, header_value)
                .body(body)
                .unwrap();
            let response = post_admission(State(state.clone()), request).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
}
