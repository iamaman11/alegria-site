#[cfg(feature = "e2e")]
#[test]
fn canonical_temporal_activity_history_wire_shape() {
    use temporalio_common::protos::temporal::api::{
        common::v1::ActivityType,
        history::v1::{ActivityTaskScheduledEventAttributes, HistoryEvent},
    };

    let event = |name: &str| {
        let attributes = ActivityTaskScheduledEventAttributes {
            activity_type: Some(ActivityType {
                name: name.to_string(),
            }),
            ..Default::default()
        };
        HistoryEvent {
            event_type: EventType::ActivityTaskScheduled as i32,
            attributes: Some(Attributes::ActivityTaskScheduledEventAttributes(attributes)),
            ..Default::default()
        }
    };
    let wire = serde_json::to_vec(&History {
        events: vec![
            event("AlegriaActivities::load_verified_support_bundle"),
            event("AlegriaActivities::run_serp_ingest_step"),
        ],
    })
    .expect("serialize SDK-generated History envelope");
    assert_eq!(
        scheduled_temporal_activities(&wire),
        vec![
            "AlegriaActivities::load_verified_support_bundle",
            "AlegriaActivities::run_serp_ingest_step",
        ]
    );
}
