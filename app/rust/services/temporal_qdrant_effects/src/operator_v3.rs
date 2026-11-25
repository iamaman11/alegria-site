//! Independent operator authorization for Qdrant's shared native owner wire.
//! The request contains *locators only*: every effect requires reciprocal
//! Temporal SERVER Histories, concrete Qdrant command verification, an exact
//! trusted route/build, and a separately committed V3 PostgreSQL admission.
//! Produced inputs cannot pass until their origin has independent proof.

use std::{env, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id, operation_dag_parent_workflow_id_v2,
    owner_dispatch::{
        owner_child_workflow_id, owner_child_workflow_id_v2, OperationDagRunV6, OwnerInvocationV1,
        OwnerResultV1, OPERATION_DAG_PARENT_QUEUE, OPERATION_DAG_PARENT_QUEUE_V2,
        OPERATION_DAG_PARENT_TYPE, OPERATION_DAG_PARENT_TYPE_V2, OWNER_OPERATION_CHILD_TYPE_V1,
    },
    owner_validation_child_workflow_id_v1, ArtifactBinding, Operation, OwnerValidationRequestV1,
    OwnerValidationResultV1, PortId, OWNER_VALIDATION_CHILD_TYPE_V1,
};
use projection_contracts::VectorProjectionSourceReceiptV1;
use qdrant_contracts::{
    verify_qdrant_owner_invocation, QdrantKeywordProjectionCommandV1,
    QdrantKeywordProjectionContractV2, QdrantProducedVectorProjectionContractV1,
    QDRANT_EFFECT_INPUT_PORT_V2, QDRANT_GENERIC_ADMISSION_SIGNAL_V1, QDRANT_GENERIC_OWNER_QUEUE_V1,
    QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use temporalio_client::{
    UntypedSignal, UntypedWorkflow, WorkflowExecutionInfo, WorkflowSignalOptions,
};
use temporalio_common::protos::temporal::api::history::v1::{history_event::Attributes, History};
use uuid::Uuid;

use super::{
    operator::{exact_history, start_payload},
    owner_generic_v1::validate_qdrant_owner_pre_effect_v1,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueRequestV3 {
    admission_id: Uuid,
    canonical_run_id: Uuid,
    parent_workflow_id: String,
    parent_workflow_run_id: String,
    child_workflow_id: String,
    child_workflow_run_id: String,
    expires_in_seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenericParentRevision {
    V1,
    V2,
}

impl GenericParentRevision {
    fn for_request(request: &IssueRequestV3) -> Result<Self> {
        if request.parent_workflow_id
            == operation_dag_parent_workflow_id(&request.canonical_run_id.to_string())
        {
            Ok(Self::V1)
        } else if request.parent_workflow_id
            == operation_dag_parent_workflow_id_v2(&request.canonical_run_id.to_string())
        {
            Ok(Self::V2)
        } else {
            bail!("Qdrant operator rejects unknown or relabelled Temporal DAG parent identity")
        }
    }

    fn temporal_type(self) -> &'static str {
        match self {
            Self::V1 => OPERATION_DAG_PARENT_TYPE,
            Self::V2 => OPERATION_DAG_PARENT_TYPE_V2,
        }
    }

    fn temporal_queue(self) -> &'static str {
        match self {
            Self::V1 => OPERATION_DAG_PARENT_QUEUE,
            Self::V2 => OPERATION_DAG_PARENT_QUEUE_V2,
        }
    }
}

struct AttestedV3 {
    invocation: OwnerInvocationV1,
    command: QdrantKeywordProjectionCommandV1,
    parent_bytes: Vec<u8>,
    child_bytes: Vec<u8>,
    parent_child_started_event_id: i64,
    parent_child_initiated_event_id: i64,
    /// Independently fetched parent/producer server History plus durable
    /// PG-issued neutral certificate, never caller-provided dispatch data.
    produced_source: Option<ProducedSourceProofV1>,
}

/// Producer proof captured from two *server-observed* Temporal Histories.
/// This is still insufficient without the independently issued durable
/// artifact certificate checked against the operator's neutral trust ledger.
struct ProducedSourceProofV1 {
    receipt_id: Uuid,
    source_hash: String,
    projection: serde_json::Value,
    operation_instance: String,
    producer_child_workflow_id: String,
    producer_child_workflow_run_id: String,
    producer_execution_key: String,
}

fn exactly_one_payload<'a>(
    payloads: Option<&'a temporalio_common::protos::temporal::api::common::v1::Payloads>,
    label: &str,
) -> Result<&'a [u8]> {
    let payloads = payloads.context(format!("{label} omitted result payloads"))?;
    ensure!(
        payloads.payloads.len() == 1,
        "{label} is not exactly one typed payload"
    );
    Ok(payloads.payloads[0].data.as_slice())
}

fn verify_validation_result_wire(
    request: &OwnerValidationRequestV1,
    result_bytes: &[u8],
) -> Result<()> {
    let result: OwnerValidationResultV1 = serde_json::from_slice(result_bytes)?;
    result.verify_success(request)?;
    Ok(())
}

/// V2 never permits a Qdrant effect Child unless EVERY DAG owner finished
/// its pure ValidateOnly Child before the first provider effect was even
/// initiated. The Qdrant operator independently verifies every owner's
/// typed validation result and reciprocal server History. Only Qdrant's own
/// concrete command is re-decoded here; foreign commands belong to owners.
async fn attest_v2_validation_barrier(
    client: &temporalio_client::Client,
    namespace: &str,
    parent: &History,
    request: &IssueRequestV3,
    effect: &AttestedV3,
) -> Result<()> {
    let run: OperationDagRunV6 = serde_json::from_slice(&effect.parent_bytes)?;
    let first_effect_id = parent
        .events
        .iter()
        .filter_map(|event| {
            if let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attrs)) =
                &event.attributes
            {
                if attrs.workflow_type.as_ref().map(|ty| ty.name.as_str())
                    == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                {
                    return Some(event.event_id);
                }
            }
            None
        })
        .min()
        .context("V2 parent server History contains no effect ChildInitiated")?;
    let validations = parent
        .events
        .iter()
        .filter_map(|event| {
            if let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attrs)) =
                &event.attributes
            {
                if attrs.workflow_type.as_ref().map(|ty| ty.name.as_str())
                    == Some(OWNER_VALIDATION_CHILD_TYPE_V1)
                {
                    return Some((event.event_id, attrs));
                }
            }
            None
        })
        .collect::<Vec<_>>();
    ensure!(
        validations.len() == run.plan.nodes.len(),
        "V2 parent did not initiate exactly one owner ValidateOnly Child per DAG operation"
    );
    let expected_validation_ids = run
        .plan
        .nodes
        .iter()
        .map(|node| {
            owner_validation_child_workflow_id_v1(
                &effect.invocation.parent,
                &run.plan.hash,
                &node.instance,
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    let actual_validation_ids = validations
        .iter()
        .map(|(_, attrs)| attrs.workflow_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(
        expected_validation_ids.len() == run.plan.nodes.len()
            && actual_validation_ids == expected_validation_ids,
        "V2 validation wave has duplicate, missing or foreign owner ValidateChild IDs"
    );
    for (id, initiated) in &validations {
        ensure!(
            *id < first_effect_id,
            "V2 validation Child began after first effect"
        );
        let completed = parent
            .events
            .iter()
            .filter_map(|event| {
                if let Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(attrs)) =
                    &event.attributes
                {
                    if attrs.initiated_event_id == *id {
                        return Some((event.event_id, attrs));
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        ensure!(
            completed.len() == 1 && completed[0].0 < first_effect_id,
            "V2 effect started before every owner validation completed"
        );

        // The own Qdrant validation is checked below, including its concrete
        // command. Check every OTHER owner here; successful completion alone
        // does not establish that the typed result accepted the exact request.
        let node = run
            .plan
            .nodes
            .iter()
            .find(|node| {
                owner_validation_child_workflow_id_v1(
                    &effect.invocation.parent,
                    &run.plan.hash,
                    &node.instance,
                ) == initiated.workflow_id
            })
            .context("V2 validation Child is not in the immutable plan")?;
        if node.instance == effect.invocation.checked_dispatch.instance {
            continue;
        }
        let owner_command = run
            .owner_commands
            .get(&node.instance)
            .context("V2 foreign validation has no pinned owner command")?;
        let matching_routes = run
            .pinned_routes
            .iter()
            .filter(|route| {
                route.operation == node.operation && route.owner == owner_command.owner
            })
            .collect::<Vec<_>>();
        ensure!(
            matching_routes.len() == 1
                && matching_routes[0].contract_hash == owner_command.contract_hash
                && initiated.workflow_type.as_ref().map(|workflow| workflow.name.as_str())
                    == Some(OWNER_VALIDATION_CHILD_TYPE_V1)
                && initiated.task_queue.as_ref().map(|queue| queue.name.as_str())
                    == Some(matching_routes[0].task_queue.as_str()),
            "V2 foreign ValidateOnly Child used an untrusted route or Task Queue"
        );
        let initiated_bytes = exactly_one_payload(
            initiated.input.as_ref(),
            "foreign ValidateOnly initiated input",
        )?;
        let validated: OwnerValidationRequestV1 = serde_json::from_slice(initiated_bytes)?;
        validated.validate_shape()?;
        ensure!(
            serde_json::to_vec(&validated)? == initiated_bytes
                && validated.parent == effect.invocation.parent
                && validated.plan_hash == run.plan.hash
                && validated.scope == run.plan.scope
                && validated.instance == node.instance
                && validated.operation == node.operation
                && validated.owner == owner_command.owner
                && validated.contract_hash == owner_command.contract_hash
                && validated.command_bytes == owner_command.command_bytes,
            "V2 foreign ValidateOnly Child request differs from exact parent plan"
        );
        ensure!(
            validated.external_inputs.len() + validated.produced_input_ports.len()
                == node.inputs.len(),
            "V2 foreign ValidateOnly contains extra or missing input ports"
        );
        for (port, binding) in &node.inputs {
            match binding {
                ArtifactBinding::External { artifact } => ensure!(
                    validated.external_inputs.get(port) == run.admitted.get(artifact)
                        && !validated.produced_input_ports.contains(port),
                    "V2 foreign ValidateOnly External bytes differ from sealed DAG"
                ),
                ArtifactBinding::Produced { .. } => ensure!(
                    validated.produced_input_ports.contains(port)
                        && !validated.external_inputs.contains_key(port),
                    "V2 foreign ValidateOnly Produced port differs from sealed DAG"
                ),
            }
        }
        let started = parent
            .events
            .iter()
            .filter_map(|event| {
                if let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                    &event.attributes
                {
                    if attrs.initiated_event_id == *id {
                        return Some((event.event_id, attrs));
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        ensure!(
            started.len() == 1,
            "V2 foreign ValidateOnly Child did not start exactly once"
        );
        let child_identity = started[0]
            .1
            .workflow_execution
            .as_ref()
            .context("V2 foreign ValidateOnly ChildStarted omitted execution identity")?;
        ensure!(
            child_identity.workflow_id == initiated.workflow_id
                && !child_identity.run_id.is_empty()
                && completed[0].1.started_event_id == started[0].0
                && completed[0].1.workflow_execution.as_ref().is_some_and(|execution| {
                    execution.workflow_id == child_identity.workflow_id
                        && execution.run_id == child_identity.run_id
                }),
            "V2 foreign ValidateOnly ChildStarted/Completed identities disagree"
        );
        let child_history = exact_history(
            client,
            namespace,
            &child_identity.workflow_id,
            &child_identity.run_id,
        )
        .await?;
        let child_input = start_payload(
            &child_history,
            OWNER_VALIDATION_CHILD_TYPE_V1,
            &matching_routes[0].task_queue,
        )?;
        ensure!(
            child_input.as_slice() == initiated_bytes,
            "V2 foreign ValidateOnly parent/child server start bytes differ"
        );
        let Some(Attributes::WorkflowExecutionStartedEventAttributes(child_start)) =
            &child_history
                .events
                .first()
                .context("V2 foreign ValidateOnly Child has no server start")?
                .attributes
        else {
            bail!("V2 foreign ValidateOnly Child lacks server WorkflowStarted");
        };
        ensure!(
            child_start.parent_workflow_execution.as_ref().is_some_and(|parent| {
                parent.workflow_id == request.parent_workflow_id
                    && parent.run_id == request.parent_workflow_run_id
            }) && child_start.parent_initiated_event_id == *id,
            "V2 foreign ValidateOnly child is not reciprocally parent-attested"
        );
        let Some(Attributes::WorkflowExecutionCompletedEventAttributes(child_done)) =
            &child_history
                .events
                .last()
                .context("V2 foreign ValidateOnly Child has no terminal event")?
                .attributes
        else {
            bail!("V2 foreign ValidateOnly Child did not complete successfully");
        };
        let result_bytes = exactly_one_payload(
            completed[0].1.result.as_ref(),
            "foreign parent ValidateOnly result",
        )?;
        ensure!(
            result_bytes == exactly_one_payload(
                child_done.result.as_ref(), "foreign child ValidateOnly result",
            )?,
            "V2 foreign ValidateOnly parent/child server results differ"
        );
        verify_validation_result_wire(&validated, result_bytes)?;
    }

    let expected_id = owner_validation_child_workflow_id_v1(
        &effect.invocation.parent,
        &effect.invocation.checked_dispatch.plan_hash,
        &effect.invocation.checked_dispatch.instance,
    );
    let matching = validations
        .iter()
        .filter(|(_, attrs)| attrs.workflow_id == expected_id)
        .collect::<Vec<_>>();
    ensure!(
        matching.len() == 1,
        "V2 parent omitted unique Qdrant ValidateOnly Child for this effect"
    );
    let (initiated_id, initiated) = matching[0];
    ensure!(
        initiated
            .task_queue
            .as_ref()
            .map(|queue| queue.name.as_str())
            == Some(QDRANT_GENERIC_OWNER_QUEUE_V1),
        "Qdrant ValidateOnly Child was routed to another owner queue"
    );
    let validation_started = parent
        .events
        .iter()
        .filter_map(|event| {
            if let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                &event.attributes
            {
                if attrs.initiated_event_id == *initiated_id {
                    return Some(attrs);
                }
            }
            None
        })
        .collect::<Vec<_>>();
    ensure!(
        validation_started.len() == 1,
        "Qdrant ValidateOnly Child lacks exact parent server ChildStarted identity"
    );
    let validation_identity = validation_started[0]
        .workflow_execution
        .as_ref()
        .context("Qdrant ValidateOnly parent ChildStarted omitted Run ID")?;
    ensure!(
        validation_identity.workflow_id == expected_id && !validation_identity.run_id.is_empty(),
        "Qdrant ValidateOnly parent ChildStarted identity is wrong"
    );
    let validation_history =
        exact_history(client, namespace, &expected_id, &validation_identity.run_id).await?;
    let input = start_payload(
        &validation_history,
        OWNER_VALIDATION_CHILD_TYPE_V1,
        QDRANT_GENERIC_OWNER_QUEUE_V1,
    )?;
    ensure!(
        exactly_one_payload(initiated.input.as_ref(), "Qdrant ValidateOnly initiation")?
            == input.as_slice(),
        "Qdrant ValidateOnly parent initiation differs from child start History"
    );
    let validated: OwnerValidationRequestV1 = serde_json::from_slice(&input)?;
    validated.validate_shape()?;
    ensure!(
        validated.parent == effect.invocation.parent
            && validated.plan_hash == effect.invocation.checked_dispatch.plan_hash
            && validated.instance == effect.invocation.checked_dispatch.instance
            && validated.operation == effect.invocation.checked_dispatch.operation
            && validated.owner == effect.invocation.checked_dispatch.owner
            && validated.command_bytes == effect.invocation.command_bytes,
        "Qdrant ValidateOnly command/owner/run does not match exact effect Child"
    );
    let node = run
        .plan
        .nodes
        .iter()
        .find(|node| node.instance == validated.instance)
        .context("Qdrant ValidateOnly node absent from immutable plan")?;
    for (port, binding) in &node.inputs {
        match binding {
            ArtifactBinding::External { artifact } => {
                ensure!(
                    validated.external_inputs.get(port) == run.admitted.get(artifact)
                        && !validated.produced_input_ports.contains(port),
                    "ValidateOnly External artifact differs from canonical DAG admission"
                );
            }
            ArtifactBinding::Produced { .. } => {
                ensure!(
                    validated.produced_input_ports.contains(port)
                        && !validated.external_inputs.contains_key(port),
                    "ValidateOnly Produced port incorrectly materialized before producer effect"
                );
            }
        }
    }
    validate_qdrant_owner_pre_effect_v1(&validated)
        .context("Qdrant own ValidateOnly Child didn't validate its actual typed command")?;
    let completed = parent
        .events
        .iter()
        .filter_map(|event| {
            if let Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(attrs)) =
                &event.attributes
            {
                if attrs.initiated_event_id == *initiated_id {
                    return Some(attrs);
                }
            }
            None
        })
        .collect::<Vec<_>>();
    ensure!(
        completed.len() == 1
            && completed[0]
                .workflow_execution
                .as_ref()
                .is_some_and(|id| id.workflow_id == validation_identity.workflow_id
                    && id.run_id == validation_identity.run_id),
        "Qdrant ValidateOnly completed another child run"
    );
    let result_bytes = exactly_one_payload(
        completed[0].result.as_ref(),
        "Qdrant parent ValidateOnly completed",
    )?;
    let last_event = validation_history
        .events
        .last()
        .context("Qdrant ValidateOnly child History empty")?;
    let Some(Attributes::WorkflowExecutionCompletedEventAttributes(actual)) =
        &last_event.attributes
    else {
        bail!("Qdrant ValidateOnly child never completed successfully");
    };
    ensure!(
        result_bytes
            == exactly_one_payload(
                actual.result.as_ref(),
                "Qdrant ValidateOnly child completed"
            )?,
        "Qdrant ValidateOnly result differs between parent and child server Histories"
    );
    verify_validation_result_wire(&validated, result_bytes)?;
    let start = validation_history
        .events
        .first()
        .context("Qdrant ValidateOnly child History has no server start")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(started)) = &start.attributes
    else {
        bail!("Qdrant ValidateOnly child first event not server WorkflowStarted");
    };
    ensure!(
        started
            .parent_workflow_execution
            .as_ref()
            .is_some_and(|id| id.workflow_id == request.parent_workflow_id
                && id.run_id == request.parent_workflow_run_id)
            && started.parent_initiated_event_id == *initiated_id,
        "Qdrant ValidateOnly child has forged parent linkage"
    );
    Ok(())
}

async fn attest_produced_source_v1(
    client: &temporalio_client::Client,
    namespace: &str,
    parent: &History,
    request: &IssueRequestV3,
    proof: &AttestedV3,
) -> Result<Option<ProducedSourceProofV1>> {
    let contract = QdrantProducedVectorProjectionContractV1::contract();
    if proof.invocation.checked_dispatch.operation != contract.reference {
        return Ok(None);
    }
    let input_port = PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1);
    let bound = proof
        .invocation
        .bound_inputs
        .get(&input_port)
        .context("Produced Qdrant child has no exact vector projection input")?;
    let evidence = bound
        .produced_evidence
        .as_ref()
        .context("Produced Qdrant child lacks completed source evidence")?;
    let (producer_instance, producer_port) = match &bound.source {
        ArtifactBinding::Produced {
            producer,
            port,
            artifact,
        } if *artifact == bound.artifact_id
            && producer == &evidence.producer
            && port == &evidence.port =>
        {
            (producer, port)
        }
        _ => bail!("Produced Qdrant input is not an exact producer-port binding"),
    };
    let run: OperationDagRunV6 = serde_json::from_slice(&proof.parent_bytes)?;
    let producer_node = run
        .plan
        .nodes
        .iter()
        .find(|node| &node.instance == producer_instance)
        .context("Produced Qdrant source operation missing from original parent plan")?;
    ensure!(
        producer_node.outputs.get(producer_port) == Some(&bound.artifact_id),
        "source operation does not produce the exact referenced artifact ID"
    );
    let producer_route = run
        .pinned_routes
        .iter()
        .filter(|route| route.operation == producer_node.operation)
        .collect::<Vec<_>>();
    ensure!(
        producer_route.len() == 1
            && producer_route[0].workflow_type == OWNER_OPERATION_CHILD_TYPE_V1
            && !producer_route[0].task_queue.trim().is_empty()
            && !producer_route[0].build_id.trim().is_empty(),
        "Produced source has no unique pinned independent owner route"
    );

    // Reciprocal producer identity is discovered only from server parent
    // ChildStarted, never from a caller-provided upstream run_id.
    let started = parent
        .events
        .iter()
        .filter_map(|event| {
            if let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                &event.attributes
            {
                if attrs
                    .workflow_execution
                    .as_ref()
                    .map(|id| id.workflow_id.as_str())
                    == Some(evidence.child_workflow_id.as_str())
                {
                    return Some((event.event_id, attrs));
                }
            }
            None
        })
        .collect::<Vec<_>>();
    ensure!(
        started.len() == 1,
        "Produced source has no unique actual parent ChildStarted event"
    );
    let (producer_started_event_id, producer_started) = started[0];
    let producer_identity = producer_started
        .workflow_execution
        .as_ref()
        .context("Produced parent ChildStarted event omitted execution identity")?;
    ensure!(
        !producer_identity.run_id.is_empty()
            && producer_identity.workflow_id == evidence.child_workflow_id,
        "Produced child has no server-selected Run ID"
    );
    let completed = parent
        .events
        .iter()
        .filter_map(|event| {
            if let Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(attrs)) =
                &event.attributes
            {
                if attrs.initiated_event_id == producer_started.initiated_event_id {
                    return Some((event.event_id, attrs));
                }
            }
            None
        })
        .collect::<Vec<_>>();
    ensure!(
        completed.len() == 1
            && completed[0].0 < proof.parent_child_initiated_event_id
            && producer_started_event_id < completed[0].0,
        "Produced source must complete before Qdrant child is even initiated in server History"
    );
    let (_, producer_completed) = completed[0];
    ensure!(
        producer_completed
            .workflow_execution
            .as_ref()
            .is_some_and(|id| id.workflow_id == producer_identity.workflow_id
                && id.run_id == producer_identity.run_id),
        "Produced source parent completion has an alien child execution identity"
    );

    let source_history = exact_history(
        client,
        namespace,
        &producer_identity.workflow_id,
        &producer_identity.run_id,
    )
    .await?;
    let source_start_bytes = start_payload(
        &source_history,
        OWNER_OPERATION_CHILD_TYPE_V1,
        &producer_route[0].task_queue,
    )?;
    let source_invocation: OwnerInvocationV1 = serde_json::from_slice(&source_start_bytes)
        .context("Produced source did not start with common typed OwnerInvocationV1")?;
    ensure!(
        source_invocation.parent == proof.invocation.parent
            && source_invocation.checked_dispatch.instance == *producer_instance
            && source_invocation.checked_dispatch.operation == producer_node.operation
            && source_invocation.checked_dispatch.owner == producer_route[0].owner,
        "Produced source child is not owned by its pinned DAG operation"
    );
    let initiated = parent
        .events
        .iter()
        .filter_map(|event| {
            if let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(attrs)) =
                &event.attributes
            {
                if event.event_id == producer_started.initiated_event_id {
                    return Some(attrs);
                }
            }
            None
        })
        .collect::<Vec<_>>();
    ensure!(
        initiated.len() == 1
            && initiated[0].workflow_id == producer_identity.workflow_id
            && initiated[0]
                .task_queue
                .as_ref()
                .map(|queue| queue.name.as_str())
                == Some(producer_route[0].task_queue.as_str()),
        "Produced source initiation is not from its pinned owner route"
    );
    ensure!(
        exactly_one_payload(initiated[0].input.as_ref(), "Produced source initiation")?
            == source_start_bytes.as_slice(),
        "Produced source parent initiation and child start inputs differ"
    );
    let source_started = source_history
        .events
        .first()
        .context("Produced source child server History empty")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(source_started_attrs)) =
        &source_started.attributes
    else {
        bail!("Produced source child did not begin with server WorkflowExecutionStarted");
    };
    ensure!(
        source_started_attrs
            .parent_workflow_execution
            .as_ref()
            .is_some_and(|id| id.workflow_id == request.parent_workflow_id
                && id.run_id == request.parent_workflow_run_id)
            && source_started_attrs.parent_initiated_event_id
                == producer_started.initiated_event_id,
        "Produced child server History is not reciprocal to the generic DAG parent"
    );
    let source_finish = source_history
        .events
        .last()
        .context("Produced child lacks terminal server event")?;
    let Some(Attributes::WorkflowExecutionCompletedEventAttributes(attrs)) =
        &source_finish.attributes
    else {
        bail!("Produced source child has no authoritative completed server result");
    };
    let source_result_bytes = exactly_one_payload(attrs.result.as_ref(), "Produced child result")?;
    let parent_result_bytes = exactly_one_payload(
        producer_completed.result.as_ref(),
        "Produced parent ChildCompleted result",
    )?;
    ensure!(
        source_result_bytes == parent_result_bytes,
        "Produced source result differs between parent and child server Histories"
    );
    let source_result: OwnerResultV1 = serde_json::from_slice(source_result_bytes)
        .context("Produced source returned non-typed child result")?;
    proof
        .invocation
        .verify_produced_input_from_observed_completion(
            &input_port,
            &source_invocation,
            &source_result,
        )
        .context("Produced Qdrant input disagrees with independently observed source completion")?;

    let receipt: VectorProjectionSourceReceiptV1 = serde_json::from_slice(
        &source_result.receipt_bytes,
    )
    .context("Produced source durable receipt is not the versioned projection certificate")?;
    ensure!(
        receipt.child_workflow_run_id == producer_identity.run_id
            && receipt.execution_key == source_invocation.checked_dispatch.execution_key
            && !receipt.source_hash.trim().is_empty(),
        "Produced source reported a receipt for a different owner execution"
    );
    let projection = serde_json::from_slice::<serde_json::Value>(
        &source_result
            .outputs
            .get(producer_port)
            .context("Produced source result omitted declared vector projection")?
            .payload,
    )
    .context("Produced source returned malformed portable projection JSON")?;

    Ok(Some(ProducedSourceProofV1 {
        receipt_id: receipt.receipt_id,
        source_hash: receipt.source_hash,
        projection,
        operation_instance: source_invocation
            .checked_dispatch
            .instance
            .as_str()
            .to_owned(),
        producer_child_workflow_id: producer_identity.workflow_id.clone(),
        producer_child_workflow_run_id: producer_identity.run_id.clone(),
        producer_execution_key: source_invocation.checked_dispatch.execution_key,
    }))
}

fn verify_trusted_temporal_url(candidate: &str) -> Result<()> {
    // A prefix test is insufficient: `http://127.0.0.1:pw@remote.example`
    // has a remote host despite its loopback-looking prefix. Parse the URL
    // and reject credentials, alternate paths and every non-loopback host.
    let parsed =
        url::Url::parse(candidate).context("generic Qdrant operator Temporal URL is malformed")?;
    ensure!(
        parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
            && parsed.port().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "generic Qdrant operator requires exact loopback Temporal URL without userinfo or redirects"
    );
    Ok(())
}

fn attest_pair_v3(
    parent: &History,
    child: &History,
    request: &IssueRequestV3,
    trusted_build_id: &str,
) -> Result<AttestedV3> {
    let parent_revision = GenericParentRevision::for_request(request)?;
    let parent_bytes = start_payload(
        parent,
        parent_revision.temporal_type(),
        parent_revision.temporal_queue(),
    )?;
    let child_bytes = start_payload(
        child,
        OWNER_OPERATION_CHILD_TYPE_V1,
        QDRANT_GENERIC_OWNER_QUEUE_V1,
    )?;
    let run: OperationDagRunV6 = serde_json::from_slice(&parent_bytes)
        .context("generic DAG parent server History contains an invalid typed run")?;
    ensure!(
        run.schema_revision == 6
            && run.plan.hash == run.plan.canonical_hash()?
            && run.plan.scope.run_id == request.canonical_run_id.to_string(),
        "generic DAG parent History has an invalid plan hash or run identity"
    );
    let invocation: OwnerInvocationV1 = serde_json::from_slice(&child_bytes)
        .context("generic Qdrant child server History has an invalid owner invocation")?;
    let command = verify_qdrant_owner_invocation(&invocation)
        .context("generic Qdrant child command is foreign, malformed, or Produced without proof")?;
    let contract = if invocation.checked_dispatch.operation
        == QdrantProducedVectorProjectionContractV1::contract().reference
    {
        QdrantProducedVectorProjectionContractV1::contract()
    } else {
        QdrantKeywordProjectionContractV2::contract()
    };
    ensure!(
        command.admission_id == request.admission_id
            && invocation.parent.workflow_id == request.parent_workflow_id
            && invocation.parent.run_id == request.parent_workflow_run_id
            && invocation.checked_dispatch.scope == run.plan.scope
            && invocation.checked_dispatch.plan_hash == run.plan.hash
            && invocation.checked_dispatch.operation == contract.reference
            && invocation.checked_dispatch.owner == contract.owner
            && invocation.checked_dispatch.effects == contract.effects
            && request.child_workflow_id
                == match parent_revision {
                    GenericParentRevision::V1 =>
                        owner_child_workflow_id(&invocation.checked_dispatch.execution_key),
                    GenericParentRevision::V2 =>
                        owner_child_workflow_id_v2(&invocation.checked_dispatch.execution_key),
                },
        "Qdrant child is not bound to exact checked run, owner and immutable parent"
    );
    let contract_hash = contract.fingerprint()?;
    ensure!(
        run.plan
            .pinned_contracts
            .iter()
            .filter(|p| { p.reference == contract.reference && p.fingerprint == contract_hash })
            .count()
            == 1,
        "concrete Qdrant operation contract/fingerprint is not pinned in parent plan"
    );
    let routes = run
        .pinned_routes
        .iter()
        .filter(|route| route.operation == contract.reference && route.owner == contract.owner)
        .collect::<Vec<_>>();
    ensure!(
        routes.len() == 1,
        "exactly one Qdrant route must be pinned in parent History"
    );
    let route = routes[0];
    ensure!(
        route.contract_hash == contract_hash
            && route.workflow_type == OWNER_OPERATION_CHILD_TYPE_V1
            && route.task_queue == QDRANT_GENERIC_OWNER_QUEUE_V1
            && route.build_id == trusted_build_id,
        "Qdrant route does not match independently approved owner build, type and queue"
    );
    let node = run
        .plan
        .nodes
        .iter()
        .find(|node| node.instance == invocation.checked_dispatch.instance)
        .context("checked Qdrant operation instance missing from original DAG")?;
    ensure!(
        node.operation == contract.reference && node.permitted_effects == contract.effects,
        "DAG node does not authorize exact Qdrant operation/effect"
    );
    let wire = run
        .owner_commands
        .get(&node.instance)
        .context("parent History has no own typed Qdrant command bytes")?;
    ensure!(
        wire.operation == contract.reference
            && wire.owner == contract.owner
            && wire.contract_hash == contract_hash
            && wire.command_bytes == invocation.command_bytes,
        "child command bytes do not match pinned parent Qdrant command"
    );
    let input_port =
        if contract.reference == QdrantProducedVectorProjectionContractV1::contract().reference {
            PortId::from(QDRANT_PRODUCED_VECTOR_INPUT_PORT_V1)
        } else {
            PortId::from(QDRANT_EFFECT_INPUT_PORT_V2)
        };
    let source = node
        .inputs
        .get(&input_port)
        .context("Qdrant DAG input port is missing")?;
    let input_id = match source {
        ArtifactBinding::External { artifact }
            if contract.reference == QdrantKeywordProjectionContractV2::contract().reference =>
        {
            artifact
        }
        ArtifactBinding::Produced { artifact, .. }
            if contract.reference
                == QdrantProducedVectorProjectionContractV1::contract().reference =>
        {
            artifact
        }
        _ => bail!("Qdrant operator rejects a source incompatible with the exact pinned contract"),
    };
    let bound = invocation
        .bound_inputs
        .get(&input_port)
        .context("Qdrant child omitted the declared typed command input")?;
    ensure!(
        bound.artifact_id == *input_id && bound.source == *source,
        "Qdrant exact Produced/External artifact ID or source differs from original DAG"
    );
    match source {
        ArtifactBinding::External { .. } => {
            let admitted = run
                .admitted
                .get(input_id)
                .context("Qdrant external input is absent from immutable parent admission")?;
            ensure!(
                bound.produced_evidence.is_none()
                    && bound.artifact == *admitted
                    && bound.artifact.payload == invocation.command_bytes,
                "Qdrant External input differs from immutable parent admission"
            );
        }
        ArtifactBinding::Produced { .. } => {
            ensure!(
                bound.produced_evidence.is_some() && !run.admitted.contains_key(input_id),
                "Qdrant Produced input cannot be smuggled into External admission"
            );
            // Actual producer server Histories and immutable SQL receipt are
            // fetched independently in attested_request before DB admission.
        }
    }
    ensure!(
        serde_json::to_vec(&invocation)? == child_bytes,
        "child start History differs from canonical generic owner invocation bytes"
    );
    let child_start = child
        .events
        .first()
        .context("Qdrant child server History empty")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(attrs)) = &child_start.attributes
    else {
        bail!("Qdrant child first event was not server WorkflowExecutionStarted");
    };
    let recorded_parent = attrs
        .parent_workflow_execution
        .as_ref()
        .context("Qdrant child server History has no parent identity")?;
    ensure!(
        recorded_parent.workflow_id == request.parent_workflow_id
            && recorded_parent.run_id == request.parent_workflow_run_id,
        "child server History parent identity is foreign"
    );
    let mut matching_started = None;
    let mut matching_initiated = None;
    for initiated in &parent.events {
        let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(init)) =
            &initiated.attributes
        else {
            continue;
        };
        if init.workflow_id != request.child_workflow_id {
            continue;
        }
        ensure!(
            init.workflow_type.as_ref().map(|v| v.name.as_str())
                == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                && init.task_queue.as_ref().map(|v| v.name.as_str())
                    == Some(QDRANT_GENERIC_OWNER_QUEUE_V1),
            "parent initiated an untrusted Qdrant child type or queue"
        );
        ensure!(
            matching_initiated.replace(initiated.event_id).is_none(),
            "multiple Qdrant ChildInitiated events cannot authorize one effect"
        );
        let payloads = &init
            .input
            .as_ref()
            .context("parent child initiation omitted typed input")?
            .payloads;
        ensure!(
            payloads.len() == 1 && payloads[0].data.as_slice() == child_bytes.as_slice(),
            "parent child initiation bytes differ from child server History"
        );
        for started in &parent.events {
            let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(actual)) =
                &started.attributes
            else {
                continue;
            };
            if actual.initiated_event_id != initiated.event_id {
                continue;
            }
            let actual_child = actual
                .workflow_execution
                .as_ref()
                .context("parent ChildStarted omitted child execution")?;
            ensure!(
                actual_child.workflow_id == request.child_workflow_id
                    && actual_child.run_id == request.child_workflow_run_id
                    && actual.workflow_type.as_ref().map(|v| v.name.as_str())
                        == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                    && attrs.parent_initiated_event_id == initiated.event_id,
                "reciprocal generic Qdrant child server History identity mismatch"
            );
            ensure!(
                matching_started.replace(started.event_id).is_none(),
                "multiple ChildStarted events cannot authorize one Qdrant side effect"
            );
        }
    }
    Ok(AttestedV3 {
        invocation,
        command,
        parent_bytes,
        child_bytes,
        parent_child_started_event_id: matching_started
            .context("no reciprocal Qdrant child server ChildStarted event")?,
        parent_child_initiated_event_id: matching_initiated
            .context("no unique Qdrant child server ChildInitiated event")?,
        produced_source: None,
    })
}

async fn operator_pool_v3(
    parent_revision: GenericParentRevision,
    requires_produced_certificate: bool,
) -> Result<PgPool> {
    ensure!(
        env::var("ALEGRIA_ENABLE_QDRANT_GENERIC_OPERATOR_V1").as_deref() == Ok("true"),
        "generic Qdrant operator requires distinct explicit opt-in"
    );
    let filename = env::var("QDRANT_GENERIC_OPERATOR_DATABASE_URL_FILE")
        .context("generic Qdrant operator requires an independent DSN secret file")?;
    let path = Path::new(&filename);
    ensure!(
        path.is_absolute(),
        "generic Qdrant operator secret path must be absolute"
    );
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_file(),
        "generic Qdrant operator secret must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "generic Qdrant operator secret requires 0600/0400 permissions"
        );
    }
    let url = fs::read_to_string(path)?
        .trim_end_matches(|ch| ch == '\r' || ch == '\n')
        .to_owned();
    ensure!(
        !url.is_empty() && url.trim() == url,
        "malformed generic Qdrant operator DSN"
    );
    let pool = PgPool::connect(&url).await?;
    let role = sqlx::query(
        "SELECT r.rolsuper,r.rolbypassrls,pipeline.current_session_tenant_v1() AS tenant,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v3_admissions','INSERT') AS v3_insert,
                COALESCE(has_table_privilege(session_user,
                    to_regclass('pipeline.qdrant_projection_v4_admissions'),'INSERT'),FALSE) AS v4_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','INSERT') AS v2_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','INSERT') AS v1_insert,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v2_admissions','SELECT') AS v2_read,
                has_table_privilege(session_user,'pipeline.qdrant_projection_v1_admissions','SELECT') AS v1_read,
                COALESCE(has_table_privilege(session_user,
                    to_regclass('pipeline.vector_projection_sources_v1'),'SELECT'),FALSE) AS producer_source_read,
                COALESCE(has_table_privilege(session_user,
                    to_regclass('pipeline.operation_artifact_receipts_v1'),'SELECT'),FALSE) AS cross_owner_receipt_read,
                COALESCE(has_function_privilege(session_user,
                    to_regprocedure('pipeline.operation_artifact_receipt_matches_v1(uuid,text,uuid,text,text,text,text,text,text,text,text,text,text,jsonb)'),
                    'EXECUTE'),FALSE) AS cross_owner_cert_verify,
                COALESCE(has_function_privilege(session_user,
                    to_regprocedure('pipeline.operation_artifact_receipt_matches_v2(uuid,text,uuid,text,text,text,text,text,text,text,text,text,text,jsonb)'),
                    'EXECUTE'),FALSE) AS cross_owner_cert_verify_v2,
                has_table_privilege(session_user,'pipeline.operation_dag_run_governance_v1','INSERT') AS governance_issue,
                has_function_privilege(session_user,
                    'pipeline.operation_dag_governance_matches_v1(uuid,text,text,text,text,text,text,boolean)','EXECUTE') AS governed,
                COALESCE(has_function_privilege(session_user,
                    to_regprocedure('pipeline.operation_dag_governance_matches_v2(uuid,text,text,text,text,text,text,boolean)'),
                    'EXECUTE'),FALSE) AS governed_v2,
                has_function_privilege(session_user,
                    'pipeline.verify_qdrant_operator_resignal_v3(uuid,uuid,text,text,text,text,text,text,text,bigint,bytea,bytea,bytea,text)','EXECUTE') AS resignal_allowed,
                COALESCE(has_function_privilege(session_user,
                    to_regprocedure('pipeline.verify_qdrant_operator_resignal_v4(uuid,uuid,text,text,text,text,text,text,text,bigint,bytea,bytea,bytea,text)'),
                    'EXECUTE'),FALSE) AS resignal_v4_allowed,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS v3_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v3(uuid,bytea,text)','EXECUTE') AS v3_confirm,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v2(uuid,bytea,text)','EXECUTE') AS v2_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v2(uuid,bytea,text)','EXECUTE') AS v2_confirm,
                has_function_privilege(session_user,
                    'pipeline.begin_qdrant_projection_v1(uuid,bytea)','EXECUTE') AS v1_begin,
                has_function_privilege(session_user,
                    'pipeline.confirm_qdrant_projection_v1(uuid,bytea)','EXECUTE') AS v1_confirm
           FROM pg_roles r WHERE r.rolname=session_user",
    ).fetch_one(&pool).await?;
    ensure!(
        !role.try_get::<bool, _>("rolsuper")?
            && !role.try_get::<bool, _>("rolbypassrls")?
            && match parent_revision {
                GenericParentRevision::V1 => role.try_get::<bool, _>("v3_insert")?,
                GenericParentRevision::V2 => role.try_get::<bool, _>("v4_insert")?,
            }
            && !role.try_get::<bool, _>("v2_insert")?
            && !role.try_get::<bool, _>("v1_insert")?
            && !role.try_get::<bool, _>("v2_read")?
            && !role.try_get::<bool, _>("v1_read")?
            && !role.try_get::<bool, _>("producer_source_read")?
            && !role.try_get::<bool, _>("cross_owner_receipt_read")?
            && (!requires_produced_certificate
                || match parent_revision {
                    GenericParentRevision::V1 =>
                        role.try_get::<bool, _>("cross_owner_cert_verify")?,
                    GenericParentRevision::V2 =>
                        role.try_get::<bool, _>("cross_owner_cert_verify_v2")?,
                })
            && !role.try_get::<bool, _>("governance_issue")?
            && match parent_revision {
                GenericParentRevision::V1 =>
                    role.try_get::<bool, _>("governed")?
                        && role.try_get::<bool, _>("resignal_allowed")?,
                GenericParentRevision::V2 =>
                    role.try_get::<bool, _>("governed_v2")?
                        && role.try_get::<bool, _>("resignal_v4_allowed")?,
            }
            && !role.try_get::<bool, _>("v3_begin")?
            && !role.try_get::<bool, _>("v3_confirm")?
            && !role.try_get::<bool, _>("v2_begin")?
            && !role.try_get::<bool, _>("v2_confirm")?
            && !role.try_get::<bool, _>("v1_begin")?
            && !role.try_get::<bool, _>("v1_confirm")?
            && role.try_get::<Option<String>, _>("tenant")?.as_deref()
                == Some(env::var("QDRANT_GENERIC_OPERATOR_TENANT_ID")?.as_str()),
        "generic Qdrant operator must have only tenant-bound V3 admission authority"
    );
    Ok(pool)
}

async fn attested_request(
    file: &Path,
) -> Result<(
    IssueRequestV3,
    String,
    temporalio_client::Client,
    AttestedV3,
    PgPool,
)> {
    let request: IssueRequestV3 = serde_json::from_slice(&fs::read(file)?)
        .context("V3 operator request must contain only strict server run locators")?;
    ensure!(
        !request.admission_id.is_nil()
            && !request.canonical_run_id.is_nil()
            && (1..=300).contains(&request.expires_in_seconds)
            && Uuid::parse_str(&request.parent_workflow_run_id).is_ok()
            && Uuid::parse_str(&request.child_workflow_run_id).is_ok(),
        "invalid generic Qdrant operator run locators or TTL"
    );
    let revision = GenericParentRevision::for_request(&request)?;
    if revision == GenericParentRevision::V2 {
        ensure!(
            env::var("ALEGRIA_ENABLE_QDRANT_GENERIC_V2_OPERATOR_V1").as_deref() == Ok("true"),
            "Qdrant V2-parent effect admission requires separate, explicit version opt-in"
        );
    }
    let temporal_url = env::var("QDRANT_GENERIC_OPERATOR_TEMPORAL_URL")?;
    let namespace = env::var("QDRANT_GENERIC_OPERATOR_TEMPORAL_NAMESPACE")?;
    verify_trusted_temporal_url(&temporal_url)?;
    ensure!(
        !namespace.trim().is_empty() && namespace.trim() == namespace,
        "invalid generic Qdrant Temporal namespace"
    );
    let trusted_build_id = env::var("QDRANT_GENERIC_OPERATOR_TRUSTED_BUILD_ID")
        .context("generic Qdrant operator requires exact trusted deployed owner build ID")?;
    ensure!(
        !trusted_build_id.trim().is_empty() && trusted_build_id.trim() == trusted_build_id,
        "invalid trusted Qdrant owner build ID"
    );
    let client = crate::connect_client(
        &temporal_url,
        format!("alegria-qdrant-generic-operator:{}", std::process::id()),
        &namespace,
    )
    .await?;
    let parent = exact_history(
        &client,
        &namespace,
        &request.parent_workflow_id,
        &request.parent_workflow_run_id,
    )
    .await?;
    let child = exact_history(
        &client,
        &namespace,
        &request.child_workflow_id,
        &request.child_workflow_run_id,
    )
    .await?;
    let mut attested = attest_pair_v3(&parent, &child, &request, &trusted_build_id)?;
    if revision == GenericParentRevision::V2 {
        attest_v2_validation_barrier(&client, &namespace, &parent, &request, &attested).await?;
    }
    // The deterministic DAG parent only carries an *assertion* of Produced
    // provenance. A separate Qdrant operator fetches both upstream server
    // Histories and checks the PG-issued, immutable neutral certificate before
    // it may commit its own effect admission. No Qdrant Worker invokes PG.
    let produced =
        attest_produced_source_v1(&client, &namespace, &parent, &request, &attested).await?;
    ensure!(
        attested.command.tenant_id == env::var("QDRANT_GENERIC_OPERATOR_TENANT_ID")?,
        "generic Qdrant operator is not the admitted command tenant"
    );
    // Legacy External-only routes must not acquire a new cross-owner
    // certificate capability. Require it exclusively for Produced dataflow.
    let pool = operator_pool_v3(revision, produced.is_some()).await?;
    if let Some(source) = &produced {
        let certificate_sql = if revision == GenericParentRevision::V2 {
            "SELECT pipeline.operation_artifact_receipt_matches_v2(\
                $1::uuid,$2::text,$3::uuid,$4::text,$5::text,$6::text,\
                $7::text,$8::text,$9::text,$10::text,$11::text,$12::text,\
                $13::text,$14::jsonb)"
        } else {
            "SELECT pipeline.operation_artifact_receipt_matches_v1(\
                $1::uuid,$2::text,$3::uuid,$4::text,$5::text,$6::text,\
                $7::text,$8::text,$9::text,$10::text,$11::text,$12::text,\
                $13::text,$14::jsonb)"
        };
        let verified: bool = sqlx::query_scalar(certificate_sql)
            .bind(source.receipt_id)
            .bind(&attested.command.tenant_id)
            .bind(request.canonical_run_id)
            .bind(&attested.command.context_key)
            .bind(&attested.command.keyword_key)
            .bind(attested.invocation.checked_dispatch.plan_hash.as_str())
            .bind(&source.operation_instance)
            .bind(&source.producer_execution_key)
            .bind(&request.parent_workflow_id)
            .bind(&request.parent_workflow_run_id)
            .bind(&source.producer_child_workflow_id)
            .bind(&source.producer_child_workflow_run_id)
            .bind(&source.source_hash)
            .bind(sqlx::types::Json(&source.projection))
            .fetch_one(&pool)
            .await
            .context(
                "independent Qdrant operator cannot verify immutable upstream artifact certificate",
            )?;
        ensure!(verified,
            "Produced Qdrant admission denied: exact upstream artifact receipt is missing, revoked, or foreign");
    }
    attested.produced_source = produced;
    Ok((request, namespace, client, attested, pool))
}

async fn signal_after_commit(
    client: temporalio_client::Client,
    namespace: &str,
    request: &IssueRequestV3,
) -> Result<()> {
    use temporalio_common::data_converters::RawValue;
    let info = WorkflowExecutionInfo::builder()
        .namespace(namespace.to_owned())
        .workflow_id(request.child_workflow_id.clone())
        .run_id(request.child_workflow_run_id.clone())
        .build();
    info.bind_untyped(client)
        .signal(
            UntypedSignal::<UntypedWorkflow>::new(QDRANT_GENERIC_ADMISSION_SIGNAL_V1),
            RawValue::default(),
            WorkflowSignalOptions::default(),
        )
        .await
        .context("signal exact generic Qdrant child after committed SQL admission")
}

pub(crate) async fn issue_from_file(path: &Path) -> Result<()> {
    let (request, namespace, client, proof, pool) = attested_request(path).await?;
    let revision = GenericParentRevision::for_request(&request)?;
    let command = &proof.command;
    let effect_command_bytes = if proof.invocation.checked_dispatch.operation
        == QdrantProducedVectorProjectionContractV1::contract().reference
    {
        serde_json::to_vec(command)?
    } else {
        proof.invocation.command_bytes.clone()
    };
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(match revision {
            GenericParentRevision::V1 => format!("truth-run-status|{}", request.canonical_run_id),
            GenericParentRevision::V2 => {
                format!("operation-dag-business|{}", request.canonical_run_id)
            }
        })
        .execute(&mut *tx)
        .await?;
    // Lock in the same order as the governance issuer/revoker and the V3
    // first-start SQL function. Revocation must not race between the active
    // governance predicate and commitment of a new Qdrant admission.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!(
            "operation-dag-canonical|{}",
            request.canonical_run_id
        ))
        .execute(&mut *tx)
        .await?;
    let canonical_sql = match revision {
        GenericParentRevision::V1 => {
            "SELECT context_key,tenant_id FROM pipeline.execution_runs
                WHERE run_id=$1 AND workflow_type='seo_site_build'
                  AND status IN ('verifying','pending_hitl')"
        }
        GenericParentRevision::V2 => {
            "SELECT context_key,tenant_id FROM pipeline.operation_dag_business_runs_v1
                WHERE run_id=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp()"
        }
    };
    let canonical = sqlx::query(canonical_sql)
        .bind(request.canonical_run_id)
        .fetch_one(&mut *tx)
        .await
        .context("generic Qdrant canonical execution run disappeared before admission")?;
    ensure!(
        canonical.try_get::<String, _>("tenant_id")? == command.tenant_id
            && canonical.try_get::<String, _>("context_key")? == command.context_key,
        "generic Qdrant canonical V1/V2 business run inactive or foreign"
    );
    // A generic DAG is a distinct Temporal execution from seo_site_build.
    // Only an independently operator-issued governance row, attested against
    // the actual generic parent server start History, can bind these runs.
    // No invented equality with the SEO Workflow Run ID is permissible.
    let governance_sql = match revision {
        GenericParentRevision::V1 => {
            "SELECT pipeline.operation_dag_governance_matches_v1(
            $1,$2,$3,$4,$5,$6,
            encode(public.digest($7::bytea,'sha256'),'hex'),TRUE)"
        }
        GenericParentRevision::V2 => {
            "SELECT pipeline.operation_dag_governance_matches_v2(
            $1,$2,$3,$4,$5,$6,
            encode(public.digest($7::bytea,'sha256'),'hex'),TRUE)"
        }
    };
    let governed: bool = sqlx::query_scalar(governance_sql)
        .bind(request.canonical_run_id)
        .bind(&command.tenant_id)
        .bind(&command.context_key)
        .bind(&request.parent_workflow_id)
        .bind(&request.parent_workflow_run_id)
        .bind(proof.invocation.checked_dispatch.plan_hash.as_str())
        .bind(&proof.parent_bytes)
        .fetch_one(&mut *tx)
        .await
        .context("generic Qdrant operator requires separately attested active DAG governance")?;
    ensure!(
        governed,
        "Qdrant V3 cannot issue admission without immutable generic DAG governance"
    );
    if let Some(source) = &proof.produced_source {
        // Recheck the immutable source receipt *under the same governance
        // transaction and advisory locks* that commits Qdrant admission. The
        // earlier check in attested_request is necessary for resignal but is
        // not by itself a sufficient effect-admission TOCTOU fence.
        let certificate_sql = if revision == GenericParentRevision::V2 {
            "SELECT pipeline.operation_artifact_receipt_matches_v2(\
                $1::uuid,$2::text,$3::uuid,$4::text,$5::text,$6::text,\
                $7::text,$8::text,$9::text,$10::text,$11::text,$12::text,\
                $13::text,$14::jsonb)"
        } else {
            "SELECT pipeline.operation_artifact_receipt_matches_v1(\
                $1::uuid,$2::text,$3::uuid,$4::text,$5::text,$6::text,\
                $7::text,$8::text,$9::text,$10::text,$11::text,$12::text,\
                $13::text,$14::jsonb)"
        };
        let verified: bool = sqlx::query_scalar(certificate_sql)
            .bind(source.receipt_id)
            .bind(&command.tenant_id)
            .bind(request.canonical_run_id)
            .bind(&command.context_key)
            .bind(&command.keyword_key)
            .bind(proof.invocation.checked_dispatch.plan_hash.as_str())
            .bind(&source.operation_instance)
            .bind(&source.producer_execution_key)
            .bind(&request.parent_workflow_id)
            .bind(&request.parent_workflow_run_id)
            .bind(&source.producer_child_workflow_id)
            .bind(&source.producer_child_workflow_run_id)
            .bind(&source.source_hash)
            .bind(sqlx::types::Json(&source.projection))
            .fetch_one(&mut *tx)
            .await
            .context("Produced source receipt disappeared during Qdrant admission transaction")?;
        ensure!(
            verified,
            "Produced source receipt revoked or mismatched before Qdrant effect admission"
        );
    }
    let insertion_sql = match revision {
        GenericParentRevision::V1 => {
            "INSERT INTO pipeline.qdrant_projection_v3_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
            child_started_event_id,operation_instance,plan_hash,dispatch_execution_key,
            parent_sha256,child_sha256,command_sha256,command_bytes,expires_at)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,
           encode(public.digest($14::bytea,'sha256'),'hex'),
           encode(public.digest($15::bytea,'sha256'),'hex'),
           encode(public.digest($16::bytea,'sha256'),'hex'),$16,
           clock_timestamp()+($17::integer*interval '1 second')
         FROM pipeline.execution_runs er
         WHERE er.run_id=$2 AND er.tenant_id=$3 AND er.context_key=$4
           AND er.workflow_type='seo_site_build'
           AND er.status IN ('verifying','pending_hitl')
           AND pipeline.operation_dag_governance_matches_v1(
               $2,$3,$4,$6,$7,$12,
               encode(public.digest($14::bytea,'sha256'),'hex'),TRUE)"
        }
        GenericParentRevision::V2 => {
            "INSERT INTO pipeline.qdrant_projection_v4_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
            child_started_event_id,operation_instance,plan_hash,dispatch_execution_key,
            parent_sha256,child_sha256,command_sha256,command_bytes,expires_at)
         SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,
           encode(public.digest($14::bytea,'sha256'),'hex'),
           encode(public.digest($15::bytea,'sha256'),'hex'),
           encode(public.digest($16::bytea,'sha256'),'hex'),$16,
           clock_timestamp()+($17::integer*interval '1 second')
         FROM pipeline.operation_dag_business_runs_v1 b
         WHERE b.run_id=$2 AND b.tenant_id=$3 AND b.context_key=$4
           AND b.revoked_at IS NULL AND b.expires_at>clock_timestamp()
           AND pipeline.operation_dag_governance_matches_v2(
               $2,$3,$4,$6,$7,$12,
               encode(public.digest($14::bytea,'sha256'),'hex'),TRUE)"
        }
    };
    let inserted = sqlx::query(insertion_sql)
        .bind(request.admission_id)
        .bind(request.canonical_run_id)
        .bind(&command.tenant_id)
        .bind(&command.context_key)
        .bind(&command.keyword_key)
        .bind(&request.parent_workflow_id)
        .bind(&request.parent_workflow_run_id)
        .bind(&request.child_workflow_id)
        .bind(&request.child_workflow_run_id)
        .bind(proof.parent_child_started_event_id)
        .bind(proof.invocation.checked_dispatch.instance.as_str())
        .bind(proof.invocation.checked_dispatch.plan_hash.as_str())
        .bind(&proof.invocation.checked_dispatch.execution_key)
        .bind(&proof.parent_bytes)
        .bind(&proof.child_bytes)
        .bind(&effect_command_bytes)
        .bind(request.expires_in_seconds as i32)
        .execute(&mut *tx)
        .await
        .context("generic Qdrant V3 admission insert denied by tenant/run/History fence")?;
    ensure!(
        inserted.rows_affected() == 1,
        "V1/V2 Qdrant must insert exactly one versioned operator admission"
    );
    tx.commit().await?;
    signal_after_commit(client, &namespace, &request)
        .await
        .context("generic Qdrant SQL admission committed, signal failed; use exact resignal")
}

pub(crate) async fn resignal_from_file(path: &Path) -> Result<()> {
    let (request, namespace, client, proof, pool) = attested_request(path).await?;
    let revision = GenericParentRevision::for_request(&request)?;
    let command = &proof.command;
    let effect_command_bytes = if proof.invocation.checked_dispatch.operation
        == QdrantProducedVectorProjectionContractV1::contract().reference
    {
        serde_json::to_vec(command)?
    } else {
        proof.invocation.command_bytes.clone()
    };
    let resignal_sql = match revision {
        GenericParentRevision::V1 => {
            "SELECT pipeline.verify_qdrant_operator_resignal_v3(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"
        }
        GenericParentRevision::V2 => {
            "SELECT pipeline.verify_qdrant_operator_resignal_v4(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"
        }
    };
    let matched: bool = sqlx::query_scalar(resignal_sql)
        .bind(request.admission_id)
        .bind(request.canonical_run_id)
        .bind(&command.tenant_id)
        .bind(&command.context_key)
        .bind(&command.keyword_key)
        .bind(&request.parent_workflow_id)
        .bind(&request.parent_workflow_run_id)
        .bind(&request.child_workflow_id)
        .bind(&request.child_workflow_run_id)
        .bind(proof.parent_child_started_event_id)
        .bind(&proof.parent_bytes)
        .bind(&proof.child_bytes)
        .bind(&effect_command_bytes)
        .bind(&proof.invocation.checked_dispatch.execution_key)
        .fetch_one(&pool)
        .await
        .context("generic Qdrant V3 SQL rejected absent/expired/foreign/changed admission")?;
    ensure!(
        matched,
        "generic Qdrant V3 immutable SQL History admission mismatch"
    );
    signal_after_commit(client, &namespace, &request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_v2_validation_receipt_must_accept_exact_request() {
        use operation_protocol::{
            ContractHash, OperationRef, PlanHash, RunScope, TemporalExecutionIdentityV4,
        };

        let scope = RunScope {
            tenant_id: "tenant-a".into(),
            run_id: Uuid::new_v4().to_string(),
        };
        let request = OwnerValidationRequestV1 {
            schema_revision: 1,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: operation_dag_parent_workflow_id_v2(&scope.run_id),
                run_id: Uuid::new_v4().to_string(),
            },
            plan_hash: PlanHash("a".repeat(64)),
            scope,
            instance: "foreign_node".into(),
            operation: OperationRef {
                id: "foreign.validated".into(),
                revision: 1,
            },
            owner: "foreign_owner".into(),
            contract_hash: ContractHash("foreign-contract".into()),
            command_bytes: vec![1, 2, 3],
            input_schemas: Default::default(),
            external_inputs: Default::default(),
            produced_input_ports: Default::default(),
        };
        let accepted = OwnerValidationResultV1::accepted(&request).unwrap();
        assert!(verify_validation_result_wire(
            &request,
            &serde_json::to_vec(&accepted).unwrap(),
        )
        .is_ok());

        let mut refused = accepted.clone();
        refused.accepted = false;
        assert!(verify_validation_result_wire(
            &request,
            &serde_json::to_vec(&refused).unwrap(),
        )
        .is_err());

        let mut forged = accepted;
        forged.request_hash = "different-validation-request".into();
        assert!(verify_validation_result_wire(
            &request,
            &serde_json::to_vec(&forged).unwrap(),
        )
        .is_err());
    }

    #[test]
    fn generic_qdrant_operator_never_confuses_v1_and_v2_parent_identities() {
        let run_id = Uuid::new_v4();
        let mut request = IssueRequestV3 {
            admission_id: Uuid::new_v4(),
            canonical_run_id: run_id,
            parent_workflow_id: operation_dag_parent_workflow_id(&run_id.to_string()),
            parent_workflow_run_id: Uuid::new_v4().to_string(),
            child_workflow_id: "unused-child".into(),
            child_workflow_run_id: Uuid::new_v4().to_string(),
            expires_in_seconds: 120,
        };
        assert_eq!(
            GenericParentRevision::for_request(&request).unwrap(),
            GenericParentRevision::V1
        );
        request.parent_workflow_id = operation_dag_parent_workflow_id_v2(&run_id.to_string());
        assert_eq!(
            GenericParentRevision::for_request(&request).unwrap(),
            GenericParentRevision::V2
        );
        assert_eq!(
            GenericParentRevision::V2.temporal_type(),
            OPERATION_DAG_PARENT_TYPE_V2
        );
        assert_eq!(
            GenericParentRevision::V2.temporal_queue(),
            OPERATION_DAG_PARENT_QUEUE_V2
        );
        request.parent_workflow_id = operation_dag_parent_workflow_id(&run_id.to_string());
        request.parent_workflow_id.push_str("-v2-forged");
        assert!(GenericParentRevision::for_request(&request).is_err());
    }

    #[test]
    fn generic_owner_operator_requires_exact_loopback_temporal_origin() {
        assert!(verify_trusted_temporal_url("http://127.0.0.1:7233").is_ok());
        assert!(verify_trusted_temporal_url("http://localhost:7233/").is_ok());
        for forged in [
            "http://127.0.0.1:pw@evil.example:7233",
            "http://localhost:pw@evil.example:7233",
            "https://localhost:7233",
            "http://127.0.0.1:7233/other",
            "http://localhost:7233/?fallback=https://evil.example",
            "http://localhost:7233/#evil",
            "http://0.0.0.0:7233",
            "http://127.0.0.1",
        ] {
            assert!(
                verify_trusted_temporal_url(forged).is_err(),
                "untrusted Temporal URL incorrectly accepted: {forged}"
            );
        }
    }
}
