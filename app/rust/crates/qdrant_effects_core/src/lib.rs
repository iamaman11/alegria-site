//! Experimental, opt-in second *real* provider effect (SEO keyword Qdrant
//! projection) with an independently persisted PostgreSQL unknown-outcome
//! barrier. NOT a production Temporal owner: its parent/child Workflow IDs
//! are only expected values until server-History attestation is implemented.
//!
//! Every initial RPC has a durable uncertain attempt committed BEFORE any
//! Qdrant I/O. Re-entry reconciles by an exact remote readback and NEVER
//! blindly emits another write. A missing point can mean a late first RPC.

use anyhow::{bail, ensure, Context, Result};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub use operation_protocol::QdrantKeywordProjectionCommandV1;

use vector_qdrant::projection_commands::{
    projection_readback_exact, upsert_projection, ProjectionIdentity, ProjectionUpsert,
};

const COLLECTION: &str = "seo_keyword_clusters_4";
const ENTITY_TYPE: &str = "keyword_cluster";
const VECTOR_DIMENSIONS: usize = 1024;
const EXPERIMENTAL_ENV: &str = "ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V1";

fn entity_key(command: &QdrantKeywordProjectionCommandV1) -> String {
    // One-shot PostgreSQL uniqueness and a UUID point ID prevent another
    // admission from overwriting this exact business-key projection.
    format!(
        "tenant/{}/context/{}/keyword/{}",
        command.tenant_id, command.context_key, command.keyword_key
    )
}

fn metadata(command: &QdrantKeywordProjectionCommandV1, sha256: &str) -> serde_json::Value {
    serde_json::json!({
        "tenant_id": command.tenant_id,
        "context_key": command.context_key,
        "keyword_key": command.keyword_key,
        "operator_admission_id": command.admission_id.to_string(),
        "approved_command_sha256": sha256,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QdrantProjectionOutcomeV1 {
    /// This invocation emitted the first provider RPC, read it back exactly,
    /// and committed a PostgreSQL receipt.
    Applied,
    /// Provider write was previously ambiguous; its exact point was found
    /// and a missing PostgreSQL receipt was recovered without replaying it.
    Reconciled,
    /// Durable receipt already existed and live provider bytes still match.
    AlreadyConfirmed,
}

/// Production is intentionally disabled even if the database migration exists.
/// No SQL function EXECUTE rights are granted to an Activity role by default.
pub async fn execute_admitted_qdrant_projection_v1(
    pool: &PgPool,
    admission_id: Uuid,
) -> Result<QdrantProjectionOutcomeV1> {
    ensure!(
        std::env::var(EXPERIMENTAL_ENV).as_deref() == Ok("1"),
        "Qdrant V1 external effect remains opt-in; no production Temporal route"
    );

    let row = sqlx::query(
        "SELECT tenant_id, context_key, keyword_key, command_bytes, command_sha256
           FROM pipeline.qdrant_projection_v1_admissions WHERE admission_id = $1",
    )
    .bind(admission_id)
    .fetch_one(pool)
    .await
    .context("admission absent or not readable by current tenant login")?;
    let bytes: Vec<u8> = row.try_get("command_bytes")?;
    let sha256: String = row.try_get("command_sha256")?;
    let command: QdrantKeywordProjectionCommandV1 = serde_json::from_slice(&bytes)
        .context("operator-approved Qdrant command is not valid versioned JSON")?;
    command.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        command.admission_id == admission_id
            && command.tenant_id == row.try_get::<String, _>("tenant_id")?
            && command.context_key == row.try_get::<String, _>("context_key")?
            && command.keyword_key == row.try_get::<String, _>("keyword_key")?,
        "operator admission identity differs from immutable provider command"
    );

    let state: String = sqlx::query_scalar("SELECT pipeline.begin_qdrant_projection_v1($1, $2)")
        .bind(admission_id)
        .bind(&bytes)
        .fetch_one(pool)
        .await
        .context("PostgreSQL rejected Qdrant effect admission before network I/O")?;

    let entity_key = entity_key(&command);
    let metadata = metadata(&command, &sha256);
    if state == "started" {
        // The admission's uncertain barrier committed on a separate SQL
        // statement BEFORE any provider write. On any failure, no retry.
        let identity = ProjectionIdentity::new(COLLECTION, ENTITY_TYPE, &entity_key)?;
        let upsert = ProjectionUpsert::new(
            identity,
            admission_id.to_string(),
            command.vector.clone(),
            VECTOR_DIMENSIONS as u64,
            "cosine",
            metadata.clone(),
        )?;
        upsert_projection(upsert)
            .await
            .context("unknown Qdrant RPC outcome: retain uncertain barrier, do not retry")?;
    } else if state != "uncertain" && state != "confirmed" {
        bail!("unsupported PostgreSQL Qdrant attempt state: {state}");
    }

    // Independent, full provider readback certifies both vector and metadata.
    // On a mismatch/missing point the row remains uncertain and NO RPC is
    // replayed. This avoids the delayed-first-write race after an HTTP timeout.
    let exact = projection_readback_exact(
        COLLECTION,
        &admission_id.to_string(),
        ENTITY_TYPE,
        &entity_key,
        &command.vector,
        &metadata,
    )
    .await
    .context("Qdrant exact readback unavailable: effect remains uncertain")?;
    ensure!(
        exact,
        "unknown or drifted external Qdrant outcome; manual reconciliation required"
    );

    let confirmed: bool =
        sqlx::query_scalar("SELECT pipeline.confirm_qdrant_projection_v1($1, $2)")
            .bind(admission_id)
            .bind(&bytes)
            .fetch_one(pool)
            .await
            .context(
                "provider readback succeeded but durable SQL receipt failed; reconcile only",
            )?;
    ensure!(
        confirmed,
        "PostgreSQL Qdrant receipt refused provider readback"
    );
    Ok(match state.as_str() {
        "started" => QdrantProjectionOutcomeV1::Applied,
        "uncertain" => QdrantProjectionOutcomeV1::Reconciled,
        _ => QdrantProjectionOutcomeV1::AlreadyConfirmed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_qdrant_command_rejects_untrusted_shape() {
        let mut command = QdrantKeywordProjectionCommandV1 {
            schema_revision: 1,
            admission_id: Uuid::new_v4(),
            tenant_id: "tenant-a".into(),
            context_key: "ES:tourist:BY".into(),
            keyword_key: "consular fee".into(),
            vector: vec![0.1; VECTOR_DIMENSIONS],
        };
        command.validate().unwrap();
        let admission_id = command.admission_id;
        command.admission_id = Uuid::nil();
        assert!(command.validate().is_err());
        command.admission_id = admission_id;
        command.vector[7] = f32::NAN;
        assert!(command.validate().is_err());
        command.vector[7] = 0.1;
        command.context_key = " other-tenant ".into();
        assert!(command.validate().is_err());
        command.context_key = "ES:tourist:BY".into();
        command.schema_revision = 2;
        assert!(command.validate().is_err());
        assert!(serde_json::from_str::<QdrantKeywordProjectionCommandV1>(
            r#"{"schema_revision":1,"unsupported_owner_url":"http://untrusted"}"#
        )
        .is_err());
    }
}
