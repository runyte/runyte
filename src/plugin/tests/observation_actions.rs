// SPDX-License-Identifier: MPL-2.0
use super::*;

fn source() -> Source {
    Source::ViewActions {
        view: "v:g:1".into(),
    }
}
fn state(id: u64) -> Snapshot {
    Snapshot::ViewActions {
        accepted: format!("a:{id}"),
    }
}
fn action(id: u64) -> Action {
    Action {
        id: format!("a:{id}"),
        request: format!("h:{id}"),
        command: "open".into(),
        pane: "p:g:1".into(),
        model_revision: "m:2".into(),
        query_revision: Some("qv:3".into()),
        selection_revision: "q:4".into(),
        selected_count: 2,
    }
}
fn subscribe(registry: &mut Registry, id: &str) {
    registry
        .subscribe(id.into(), vec![source()], vec![(source(), state(0))])
        .unwrap();
}
fn drain(registry: &mut Registry) -> Vec<Delivery> {
    let mut results = Vec::new();
    while let Some(delivery) = registry.peek_ready() {
        results.push(delivery);
        registry.ack_ready();
    }
    results
}

#[test]
fn actions_without_watchers_retain_no_source_or_delivery() {
    let mut registry = Registry::default();
    registry.preflight_action(&source(), &action(1)).unwrap();
    registry.record_action(&source(), action(1)).unwrap();
    assert!(registry.current.is_empty());
    assert!(registry.peek_ready().is_none());
    subscribe(&mut registry, "later");
    assert!(registry.peek_ready().is_none());
}

#[test]
fn action_fanout_is_reliable_and_correlates_exact_callback_without_duplicate_state() {
    let mut registry = Registry::default();
    subscribe(&mut registry, "one");
    subscribe(&mut registry, "two");
    registry.record_action(&source(), action(1)).unwrap();
    registry.observe(&source(), state(1)).unwrap();
    let deliveries = drain(&mut registry);
    assert_eq!(deliveries.len(), 2);
    let events: Vec<_> = deliveries
        .into_iter()
        .map(|delivery| {
            let Delivery::Action(event) = delivery else {
                panic!()
            };
            event
        })
        .collect();
    assert_eq!(events[0].revision, events[1].revision);
    assert_eq!(events[0].action, action(1));
    assert_eq!(events[0].source, source());
    assert_eq!(events[0].subscription, "one");
    assert_eq!(events[1].subscription, "two");
    let encoded = serde_json::to_value(&events[0]).unwrap();
    assert!(encoded["action"].get("rows").is_none());
    assert!(encoded["action"].get("arguments").is_none());
}

#[test]
fn suspended_actions_survive_resync_and_unsubscribe_before_acknowledgement() {
    let mut registry = Registry::default();
    subscribe(&mut registry, "sub");
    registry.record_action(&source(), action(1)).unwrap();
    registry.invalidate("sub").unwrap();
    registry.record_action(&source(), action(2)).unwrap();
    registry.resync("sub", vec![(source(), state(2))]).unwrap();
    let final_events = registry.unsubscribe("sub");
    assert_eq!(final_events.len(), 2);
    for (index, delivery) in final_events.into_iter().enumerate() {
        let Delivery::Action(event) = delivery else {
            panic!()
        };
        assert_eq!(event.action.id, format!("a:{}", index + 1));
    }
    assert!(registry.peek_ready().is_none());
    assert!(registry.current.is_empty());
}

#[test]
fn action_fanout_preflight_refuses_atomically_against_shared_reliable_capacity() {
    let mut registry = Registry::default();
    for index in 0..MAX_SUBSCRIPTIONS {
        subscribe(&mut registry, &format!("s:{index}"));
    }
    registry.record_action(&source(), action(1)).unwrap();
    registry.record_action(&source(), action(2)).unwrap();
    assert_eq!(registry.reliable.len(), MAX_RELIABLE);
    let revision = registry.next_revision;
    assert_eq!(
        registry
            .preflight_action(&source(), &action(3))
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    assert!(registry.record_action(&source(), action(3)).is_err());
    assert_eq!(registry.next_revision, revision);
    assert_eq!(registry.current[&source()].state, state(2));
    registry.ack_ready();
    assert!(registry.preflight_action(&source(), &action(3)).is_err());
    drain(&mut registry);
    registry.record_action(&source(), action(3)).unwrap();
    assert_eq!(registry.reliable.len(), MAX_SUBSCRIPTIONS);
}

#[test]
fn action_close_order_and_metadata_failures_preserve_accepted_records() {
    let mut registry = Registry::default();
    subscribe(&mut registry, "sub");
    let mut invalid = action(1);
    invalid.selected_count = super::super::view::MAX_ROWS + 1;
    assert!(registry.preflight_action(&source(), &invalid).is_err());
    invalid = action(1);
    invalid.command = "bad command".into();
    assert!(registry.preflight_action(&source(), &invalid).is_err());
    invalid = action(1);
    invalid.request = "x".repeat(129);
    assert!(registry.preflight_action(&source(), &invalid).is_err());
    assert!(
        registry
            .preflight_action(&Source::Attachment, &action(1))
            .is_err()
    );
    registry.record_action(&source(), action(1)).unwrap();
    registry.observe(&source(), Snapshot::Closed {}).unwrap();
    assert_eq!(
        registry
            .preflight_action(&source(), &action(2))
            .unwrap_err()
            .code,
        ErrorCode::Closed
    );
    let deliveries = drain(&mut registry);
    assert!(matches!(
        deliveries.as_slice(),
        [Delivery::Action(_), Delivery::Closed(_)]
    ));
}

#[test]
fn query_and_viewport_metadata_coalesce_and_legacy_view_shape_is_unchanged() {
    let legacy = Snapshot::View {
        revision: "m:1".into(),
        query: None,
    };
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::json!({"kind":"view","revision":"m:1"})
    );
    let source = Source::Viewport {
        view: "v:1".into(),
        pane: "p:1".into(),
    };
    let initial = Snapshot::Viewport {
        model_revision: None,
        visible: false,
        top: None,
        bottom: None,
    };
    let mut registry = Registry::default();
    registry
        .subscribe(
            "sub".into(),
            vec![source.clone()],
            vec![(source.clone(), initial)],
        )
        .unwrap();
    for id in [1, 2] {
        registry
            .observe(
                &source,
                Snapshot::Viewport {
                    model_revision: Some("m:2".into()),
                    visible: true,
                    top: Some(format!("row:{id}")),
                    bottom: Some(format!("row:{}", id + 5)),
                },
            )
            .unwrap();
    }
    let Some(Delivery::Changed(change)) = registry.peek_ready() else {
        panic!()
    };
    assert_eq!(change.coalesced, 1);
    assert!(
        matches!(&change.sources[0].state, Snapshot::Viewport { top: Some(top), .. } if top == "row:2")
    );
    let view = Source::View { view: "v:1".into() };
    registry
        .subscribe(
            "view".into(),
            vec![view.clone()],
            vec![(view.clone(), legacy)],
        )
        .unwrap();
    registry
        .observe(
            &view,
            Snapshot::View {
                revision: "m:1".into(),
                query: Some(super::super::view::Query {
                    revision: "qv:1".into(),
                    text: "猫".into(),
                    pending: true,
                }),
            },
        )
        .unwrap();
    assert!(registry.reliable.is_empty());
}
