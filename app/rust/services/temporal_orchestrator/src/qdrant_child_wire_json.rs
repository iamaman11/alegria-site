//! Exact native child Workflow-result JSON boundary. The provider-free DAG
//! interpreter handles only typed receipts; JSON decoding stays at this
//! individually allowlisted Temporal transport adapter, not application logic.

use operation_protocol::QdrantKeywordChildResultV1;

pub(crate) fn decode_result(value: &str) -> Result<QdrantKeywordChildResultV1, serde_json::Error> {
    serde_json::from_str(value)
}
