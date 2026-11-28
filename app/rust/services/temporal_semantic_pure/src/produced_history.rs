//! Independent Temporal server History attestation. Called only from an Activity.
use anyhow::{bail, ensure, Context, Result};
use futures::StreamExt;
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, owner_child_workflow_id_v2,
    owner_validation_child_workflow_id_v1, ArtifactBinding, OperationDagRunV6, OwnerInvocationV1,
    OwnerResultV1, OwnerValidationRequestV1, OwnerValidationResultV1, TemporalExecutionIdentityV4,
    TrustedRouteCatalog, OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
    OWNER_OPERATION_CHILD_TYPE_V1, OWNER_VALIDATION_CHILD_TYPE_V1,
};
use prost::Message;
use temporalio_client::{WorkflowExecutionInfo, WorkflowFetchHistoryOptions};
use temporalio_common::protos::temporal::api::{
    common::v1::Payloads,
    history::v1::{history_event::Attributes, History},
};

const MAX_EVENTS: usize = 10_000;
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;

async fn history(
    client: &temporalio_client::Client,
    namespace: &str,
    id: &TemporalExecutionIdentityV4,
) -> Result<History> {
    ensure!(
        !namespace.is_empty() && !id.workflow_id.is_empty() && !id.run_id.is_empty(),
        "exact namespace and execution required"
    );
    let handle = WorkflowExecutionInfo::builder()
        .namespace(namespace.to_owned())
        .workflow_id(id.workflow_id.clone())
        .run_id(id.run_id.clone())
        .build()
        .bind_untyped(client.clone());
    let mut stream = handle.fetch_history(WorkflowFetchHistoryOptions::builder().build());
    let mut events = Vec::new();
    let mut bytes = 0usize;
    while let Some(event) = stream.next().await {
        let event = event.context("fetch exact Temporal History")?;
        bytes = bytes
            .checked_add(event.encoded_len())
            .context("History size overflow")?;
        ensure!(
            events.len() < MAX_EVENTS && bytes <= MAX_HISTORY_BYTES,
            "Temporal History exceeds attestation bounds"
        );
        events.push(event);
    }
    ensure!(!events.is_empty(), "empty Temporal History");
    Ok(History { events })
}

fn payload(value: Option<&Payloads>) -> Result<&[u8]> {
    let value = value.context("missing typed payload")?;
    ensure!(
        value.payloads.len() == 1
            && !value.payloads[0].data.is_empty()
            && value.payloads[0].data.len() <= MAX_PAYLOAD_BYTES,
        "expected one bounded payload"
    );
    Ok(&value.payloads[0].data)
}

fn start<'a>(history: &'a History, workflow_type: &str, queue: &str) -> Result<(&'a temporalio_common::protos::temporal::api::history::v1::WorkflowExecutionStartedEventAttributes, &'a [u8])>{
    let event = history.events.first().context("empty history")?;
    let Some(Attributes::WorkflowExecutionStartedEventAttributes(attrs)) = &event.attributes else {
        bail!("missing WorkflowStarted")
    };
    ensure!(
        event.event_id == 1
            && attrs
                .workflow_type
                .as_ref()
                .is_some_and(|v| v.name == workflow_type)
            && attrs.task_queue.as_ref().is_some_and(|v| v.name == queue),
        "wrong workflow type or queue"
    );
    Ok((attrs, payload(attrs.input.as_ref())?))
}

/// Verify child initiation/start against independently fetched parent and child histories.
fn pair(
    parent: &History,
    child: &History,
    parent_id: &TemporalExecutionIdentityV4,
    child_id: &TemporalExecutionIdentityV4,
    workflow_type: &str,
    queue: &str,
) -> Result<i64> {
    let (started, child_bytes) = start(child, workflow_type, queue)?;
    let initiated: Vec<_> = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_id == child_id.workflow_id =>
            {
                Some((event.event_id, a))
            }
            _ => None,
        })
        .collect();
    ensure!(initiated.len() == 1, "expected unique child initiation");
    let (init_id, init) = initiated[0];
    ensure!(
        init.workflow_type
            .as_ref()
            .is_some_and(|v| v.name == workflow_type)
            && init.task_queue.as_ref().is_some_and(|v| v.name == queue)
            && payload(init.input.as_ref())? == child_bytes,
        "child initiation differs from child start"
    );
    let starts: Vec<_> = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(a))
                if a.initiated_event_id == init_id =>
            {
                Some((event.event_id, a))
            }
            _ => None,
        })
        .collect();
    ensure!(
        starts.len() == 1
            && starts[0].0 > init_id
            && starts[0]
                .1
                .workflow_execution
                .as_ref()
                .is_some_and(
                    |v| v.workflow_id == child_id.workflow_id && v.run_id == child_id.run_id
                ),
        "child start identity mismatch"
    );
    ensure!(
        started
            .parent_workflow_execution
            .as_ref()
            .is_some_and(|v| v.workflow_id == parent_id.workflow_id && v.run_id == parent_id.run_id)
            && started.parent_initiated_event_id == init_id,
        "child has different server parent"
    );
    Ok(init_id)
}

async fn validation_barrier(
    client: &temporalio_client::Client,
    namespace: &str,
    parent: &History,
    parent_id: &TemporalExecutionIdentityV4,
    run: &OperationDagRunV6,
) -> Result<()> {
    let first_effect = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_type
                    .as_ref()
                    .is_some_and(|v| v.name == OWNER_OPERATION_CHILD_TYPE_V1) =>
            {
                Some(event.event_id)
            }
            _ => None,
        })
        .min()
        .context("no owner execution initiated")?;
    let validations: Vec<_> = parent
        .events
        .iter()
        .filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_type
                    .as_ref()
                    .is_some_and(|v| v.name == OWNER_VALIDATION_CHILD_TYPE_V1) =>
            {
                Some((event.event_id, a))
            }
            _ => None,
        })
        .collect();
    ensure!(
        validations.len() == run.plan.nodes.len(),
        "incomplete validation wave"
    );
    for node in &run.plan.nodes {
        let id = owner_validation_child_workflow_id_v1(parent_id, &run.plan.hash, &node.instance);
        let matches: Vec<_> = validations
            .iter()
            .filter(|(_, a)| a.workflow_id == id)
            .collect();
        ensure!(matches.len() == 1, "missing or duplicate validation child");
        let (init_id, initiated) = *matches[0];
        ensure!(
            init_id < first_effect,
            "validation initiated after owner execution"
        );
        let route: Vec<_> = run
            .pinned_routes
            .iter()
            .filter(|r| r.operation == node.operation)
            .collect();
        ensure!(
            route.len() == 1
                && initiated
                    .task_queue
                    .as_ref()
                    .is_some_and(|v| v.name == route[0].task_queue),
            "invalid validation route"
        );
        let request_bytes = payload(initiated.input.as_ref())?;
        let request: OwnerValidationRequestV1 = serde_json::from_slice(request_bytes)?;
        request.verify_against_run(run)?;
        ensure!(
            serde_json::to_vec(&request)? == request_bytes
                && request.parent == *parent_id
                && request.instance == node.instance,
            "validation request differs from plan"
        );
        let starts: Vec<_> = parent
            .events
            .iter()
            .filter_map(|event| match &event.attributes {
                Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(a))
                    if a.initiated_event_id == init_id =>
                {
                    Some((event.event_id, a))
                }
                _ => None,
            })
            .collect();
        let completed: Vec<_> = parent
            .events
            .iter()
            .filter_map(|event| match &event.attributes {
                Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(a))
                    if a.initiated_event_id == init_id =>
                {
                    Some((event.event_id, a))
                }
                _ => None,
            })
            .collect();
        ensure!(
            starts.len() == 1
                && completed.len() == 1
                && starts[0].0 < completed[0].0
                && completed[0].0 < first_effect
                && completed[0].1.started_event_id == starts[0].0,
            "validation did not complete before execution"
        );
        let execution = starts[0]
            .1
            .workflow_execution
            .as_ref()
            .context("validation execution missing")?;
        let child_id = TemporalExecutionIdentityV4 {
            workflow_id: execution.workflow_id.clone(),
            run_id: execution.run_id.clone(),
        };
        ensure!(
            child_id.workflow_id == id
                && !child_id.run_id.is_empty()
                && completed[0]
                    .1
                    .workflow_execution
                    .as_ref()
                    .is_some_and(|v| v.workflow_id == id && v.run_id == child_id.run_id),
            "validation execution mismatch"
        );
        let child = history(client, namespace, &child_id).await?;
        pair(
            parent,
            &child,
            parent_id,
            &child_id,
            OWNER_VALIDATION_CHILD_TYPE_V1,
            &route[0].task_queue,
        )?;
        let Some(Attributes::WorkflowExecutionCompletedEventAttributes(done)) = &child
            .events
            .last()
            .context("validation history empty")?
            .attributes
        else {
            bail!("validation did not complete")
        };
        let result_bytes = payload(done.result.as_ref())?;
        ensure!(
            result_bytes == payload(completed[0].1.result.as_ref())?,
            "validation result differs across histories"
        );
        let result: OwnerValidationResultV1 = serde_json::from_slice(result_bytes)?;
        ensure!(
            serde_json::to_vec(&result)? == result_bytes,
            "noncanonical validation result"
        );
        result.verify_success(&request)?;
    }
    Ok(())
}

fn verify_plan_invocation(
    run: &OperationDagRunV6,
    invocation: &OwnerInvocationV1,
    routes: &TrustedRouteCatalog,
) -> Result<()> {
    invocation.verify_integrity(routes)?;
    let dispatch = &invocation.checked_dispatch;
    let nodes: Vec<_> = run
        .plan
        .nodes
        .iter()
        .filter(|node| node.instance == dispatch.instance)
        .collect();
    ensure!(
        nodes.len() == 1
            && nodes[0].operation == dispatch.operation
            && nodes[0].permitted_effects == dispatch.effects,
        "invocation differs from plan node"
    );
    let command = run
        .owner_commands
        .get(&dispatch.instance)
        .context("plan command missing")?;
    ensure!(
        command.operation == dispatch.operation
            && command.owner == dispatch.owner
            && command.command_bytes == invocation.command_bytes,
        "invocation command differs from plan"
    );
    ensure!(
        nodes[0].inputs.len() == invocation.bound_inputs.len(),
        "plan input count differs"
    );
    for (port, bound) in &invocation.bound_inputs {
        ensure!(
            nodes[0].inputs.get(port) == Some(&bound.source),
            "input binding differs from plan"
        );
        if let ArtifactBinding::External { artifact } = &bound.source {
            ensure!(
                run.admitted.get(artifact) == Some(&bound.artifact),
                "external artifact differs from admitted parent bytes"
            );
        }
    }
    Ok(())
}

/// Prove native V2 child identity and corroborate each Produced binding against
/// actual prior sibling completion. The supplied route catalog is installed
/// worker configuration, never a route snapshot supplied by the caller.
pub async fn attest_semantic_invocation(
    client: &temporalio_client::Client,
    namespace: &str,
    invocation: &OwnerInvocationV1,
    consumer_execution: &TemporalExecutionIdentityV4,
    routes: &TrustedRouteCatalog,
) -> Result<()> {
    let parent = history(client, namespace, &invocation.parent).await?;
    let (_, parent_bytes) = start(
        &parent,
        OPERATION_DAG_PARENT_TYPE_V2,
        OPERATION_DAG_PARENT_QUEUE_V2,
    )?;
    let run: OperationDagRunV6 = serde_json::from_slice(parent_bytes)?;
    ensure!(
        serde_json::to_vec(&run)? == parent_bytes
            && run.plan.hash == run.plan.canonical_hash()?
            && invocation.parent.workflow_id
                == operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id),
        "noncanonical V2 parent"
    );
    routes.validate_run_routes(&run)?;
    run.plan.validate(routes.catalog())?;
    verify_plan_invocation(&run, invocation, routes)?;
    let route = routes.route_for_dispatch(&invocation.checked_dispatch)?;
    ensure!(
        invocation.checked_dispatch.plan_hash == run.plan.hash
            && invocation.checked_dispatch.scope == run.plan.scope
            && consumer_execution.workflow_id
                == owner_child_workflow_id_v2(&invocation.checked_dispatch.execution_key),
        "consumer dispatch is outside parent scope"
    );
    let consumer = history(client, namespace, consumer_execution).await?;
    let (_, consumer_bytes) = start(&consumer, OWNER_OPERATION_CHILD_TYPE_V1, &route.task_queue)?;
    ensure!(
        serde_json::to_vec(invocation)? == consumer_bytes,
        "consumer invocation differs from server History"
    );
    validation_barrier(client, namespace, &parent, &invocation.parent, &run).await?;
    let consumer_initiated = pair(
        &parent,
        &consumer,
        &invocation.parent,
        consumer_execution,
        OWNER_OPERATION_CHILD_TYPE_V1,
        &route.task_queue,
    )?;
    for (port, bound) in &invocation.bound_inputs {
        let ArtifactBinding::Produced {
            producer,
            port: producer_port,
            artifact,
        } = &bound.source
        else {
            ensure!(
                bound.produced_evidence.is_none(),
                "external input has producer evidence"
            );
            continue;
        };
        let evidence = bound
            .produced_evidence
            .as_ref()
            .context("missing produced evidence")?;
        let nodes: Vec<_> = run
            .plan
            .nodes
            .iter()
            .filter(|n| &n.instance == producer)
            .collect();
        ensure!(
            nodes.len() == 1
                && nodes[0].outputs.get(producer_port) == Some(artifact)
                && artifact == &bound.artifact_id,
            "producer missing from plan"
        );
        let starts: Vec<_> = parent
            .events
            .iter()
            .filter_map(|event| match &event.attributes {
                Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(a))
                    if a.workflow_execution
                        .as_ref()
                        .is_some_and(|v| v.workflow_id == evidence.child_workflow_id) =>
                {
                    Some((event.event_id, a))
                }
                _ => None,
            })
            .collect();
        ensure!(starts.len() == 1, "missing unique producer start");
        let (started_id, started) = starts[0];
        let execution = started
            .workflow_execution
            .as_ref()
            .context("producer execution missing")?;
        let producer_id = TemporalExecutionIdentityV4 {
            workflow_id: execution.workflow_id.clone(),
            run_id: execution.run_id.clone(),
        };
        ensure!(!producer_id.run_id.is_empty(), "producer run ID missing");
        let completed: Vec<_> = parent
            .events
            .iter()
            .filter_map(|event| match &event.attributes {
                Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(a))
                    if a.initiated_event_id == started.initiated_event_id =>
                {
                    Some((event.event_id, a))
                }
                _ => None,
            })
            .collect();
        ensure!(
            completed.len() == 1
                && started_id < completed[0].0
                && completed[0].1.started_event_id == started_id
                && completed[0].0 < consumer_initiated
                && completed[0]
                    .1
                    .workflow_execution
                    .as_ref()
                    .is_some_and(|v| v.workflow_id == producer_id.workflow_id
                        && v.run_id == producer_id.run_id),
            "producer did not complete before consumer initiation"
        );
        let source = history(client, namespace, &producer_id).await?;
        let pinned: Vec<_> = run
            .pinned_routes
            .iter()
            .filter(|r| r.operation == nodes[0].operation)
            .collect();
        ensure!(pinned.len() == 1, "producer route missing");
        let (_, input) = start(
            &source,
            OWNER_OPERATION_CHILD_TYPE_V1,
            &pinned[0].task_queue,
        )?;
        let upstream: OwnerInvocationV1 = serde_json::from_slice(input)?;
        ensure!(
            serde_json::to_vec(&upstream)? == input
                && upstream.checked_dispatch.instance == *producer
                && upstream.checked_dispatch.operation == nodes[0].operation
                && upstream.parent == invocation.parent
                && producer_id.workflow_id
                    == owner_child_workflow_id_v2(&upstream.checked_dispatch.execution_key),
            "producer invocation differs from plan"
        );
        verify_plan_invocation(&run, &upstream, routes)?;
        ensure!(
            routes.route_for_dispatch(&upstream.checked_dispatch)? == pinned[0],
            "producer route is not installed"
        );
        pair(
            &parent,
            &source,
            &invocation.parent,
            &producer_id,
            OWNER_OPERATION_CHILD_TYPE_V1,
            &pinned[0].task_queue,
        )?;
        let Some(Attributes::WorkflowExecutionCompletedEventAttributes(finish)) = &source
            .events
            .last()
            .context("empty producer History")?
            .attributes
        else {
            bail!("producer not completed")
        };
        let result_bytes = payload(finish.result.as_ref())?;
        ensure!(
            result_bytes == payload(completed[0].1.result.as_ref())?,
            "producer result differs across Histories"
        );
        let result: OwnerResultV1 = serde_json::from_slice(result_bytes)?;
        ensure!(
            serde_json::to_vec(&result)? == result_bytes,
            "noncanonical producer result"
        );
        invocation.verify_produced_input_from_observed_completion(port, &upstream, &result)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use temporalio_common::protos::temporal::api::{
        common::v1::{Payload, WorkflowExecution, WorkflowType},
        history::v1::{
            ChildWorkflowExecutionStartedEventAttributes, HistoryEvent,
            StartChildWorkflowExecutionInitiatedEventAttributes,
            WorkflowExecutionStartedEventAttributes,
        },
        taskqueue::v1::TaskQueue,
    };

    #[test]
    fn rejects_forged_reciprocal_parent() {
        let parent_id = TemporalExecutionIdentityV4 {
            workflow_id: "parent".into(),
            run_id: "parent-run".into(),
        };
        let child_id = TemporalExecutionIdentityV4 {
            workflow_id: "child".into(),
            run_id: "child-run".into(),
        };
        let input = Some(Payloads {
            payloads: vec![Payload {
                data: b"{}".to_vec(),
                ..Default::default()
            }],
        });
        let queue = Some(TaskQueue {
            name: "queue".into(),
            ..Default::default()
        });
        let ty = Some(WorkflowType {
            name: OWNER_OPERATION_CHILD_TYPE_V1.into(),
        });
        let parent = History {
            events: vec![
                HistoryEvent {
                    event_id: 2,
                    attributes: Some(
                        Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(
                            StartChildWorkflowExecutionInitiatedEventAttributes {
                                workflow_id: child_id.workflow_id.clone(),
                                workflow_type: ty.clone(),
                                task_queue: queue.clone(),
                                input: input.clone(),
                                ..Default::default()
                            },
                        ),
                    ),
                    ..Default::default()
                },
                HistoryEvent {
                    event_id: 3,
                    attributes: Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(
                        ChildWorkflowExecutionStartedEventAttributes {
                            initiated_event_id: 2,
                            workflow_execution: Some(WorkflowExecution {
                                workflow_id: child_id.workflow_id.clone(),
                                run_id: child_id.run_id.clone(),
                            }),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                },
            ],
        };
        let mut child = History {
            events: vec![HistoryEvent {
                event_id: 1,
                attributes: Some(Attributes::WorkflowExecutionStartedEventAttributes(
                    WorkflowExecutionStartedEventAttributes {
                        workflow_type: ty,
                        task_queue: queue,
                        input,
                        parent_workflow_execution: Some(WorkflowExecution {
                            workflow_id: parent_id.workflow_id.clone(),
                            run_id: parent_id.run_id.clone(),
                        }),
                        parent_initiated_event_id: 2,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }],
        };
        assert!(pair(
            &parent,
            &child,
            &parent_id,
            &child_id,
            OWNER_OPERATION_CHILD_TYPE_V1,
            "queue"
        )
        .is_ok());
        if let Some(Attributes::WorkflowExecutionStartedEventAttributes(a)) =
            &mut child.events[0].attributes
        {
            a.parent_workflow_execution.as_mut().unwrap().run_id = "forged".into();
        }
        assert!(pair(
            &parent,
            &child,
            &parent_id,
            &child_id,
            OWNER_OPERATION_CHILD_TYPE_V1,
            "queue"
        )
        .is_err());
    }
}
