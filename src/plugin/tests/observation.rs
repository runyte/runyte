// SPDX-License-Identifier: MPL-2.0

use super::*;

fn buffer(index: usize) -> Source {
    Source::Buffer {
        buffer: format!("b:{index}"),
    }
}
fn snapshot(revision: usize) -> Snapshot {
    Snapshot::Buffer {
        revision: format!("r:{revision}"),
        saved_revision: Some("r:0".into()),
        name: "document".into(),
        chars: revision,
        read_only: false,
        dirty: revision != 0,
    }
}
fn subscribe(registry: &mut Registry, id: &str, count: usize) -> Vec<SourceState> {
    let sources: Vec<_> = (0..count).map(buffer).collect();
    registry
        .subscribe(
            id.into(),
            sources.clone(),
            sources
                .into_iter()
                .map(|source| (source, snapshot(0)))
                .collect(),
        )
        .unwrap()
}
fn drain(registry: &mut Registry) -> Vec<Delivery> {
    let mut deliveries = Vec::new();
    while let Some(delivery) = registry.peek_ready() {
        deliveries.push(delivery);
        registry.ack_ready();
    }
    deliveries
}

#[test]
fn baseline_latest_state_and_admission_are_atomic() {
    let mut registry = Registry::default();
    let baseline = subscribe(&mut registry, "sub", 1);
    assert_eq!(baseline[0].revision, "o:1");
    assert!(registry.peek_ready().is_none());
    registry.observe(&buffer(0), snapshot(1)).unwrap();
    registry.observe(&buffer(0), snapshot(2)).unwrap();
    let Some(Delivery::Changed(change)) = registry.peek_ready() else {
        panic!()
    };
    assert_eq!(change.coalesced, 1);
    assert_eq!(change.sources[0].state, snapshot(2));
    assert_eq!(change.sources[0].revision, "o:3");
    // Failed outbound admission leaves the same newest state available.
    let Some(Delivery::Changed(retry)) = registry.peek_ready() else {
        panic!()
    };
    assert_eq!(change.sources, retry.sources);
    registry.ack_ready();
    assert!(registry.peek_ready().is_none());
    let error = registry
        .subscribe(
            "bad".into(),
            vec![buffer(0)],
            vec![(
                buffer(0),
                Snapshot::View {
                    revision: "m:1".into(),
                },
            )],
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.current[&buffer(0)].revision, "o:3");
}

#[test]
fn overflow_requires_resync_but_close_remains_reliable() {
    let mut registry = Registry::default();
    subscribe(&mut registry, "sub", MAX_PENDING + 1);
    for index in 0..=MAX_PENDING {
        registry.observe(&buffer(index), snapshot(1)).unwrap();
    }
    assert!(registry.is_suspended("sub"));
    assert!(matches!(
        registry.peek_ready(),
        Some(Delivery::ResyncRequired(_))
    ));
    registry.ack_ready();
    registry.observe(&buffer(0), snapshot(2)).unwrap();
    assert!(registry.peek_ready().is_none());
    registry.observe(&buffer(0), Snapshot::Closed {}).unwrap();
    assert!(matches!(registry.peek_ready(), Some(Delivery::Closed(_))));
    registry.ack_ready();
    let mut baseline: Vec<_> = (0..=MAX_PENDING)
        .map(|index| (buffer(index), snapshot(2)))
        .collect();
    baseline[0].1 = Snapshot::Closed {};
    let states = registry.resync("sub", baseline).unwrap();
    assert!(!registry.is_suspended("sub"));
    assert_eq!(states.len(), MAX_PENDING + 1);
    assert!(registry.peek_ready().is_none());
    registry.observe(&buffer(1), snapshot(3)).unwrap();
    assert!(matches!(registry.peek_ready(), Some(Delivery::Changed(_))));
}

#[test]
fn save_job_and_attachment_transitions_do_not_coalesce() {
    let mut registry = Registry::default();
    let job = Source::Job { job: "j:1".into() };
    registry
        .subscribe(
            "sub".into(),
            vec![buffer(0), job.clone(), Source::Attachment],
            vec![
                (buffer(0), snapshot(0)),
                (
                    job.clone(),
                    Snapshot::Job {
                        state: JobState::Running,
                        progress: 0,
                    },
                ),
                (
                    Source::Attachment,
                    Snapshot::Attachment {
                        attached: true,
                        generation: "1".into(),
                    },
                ),
            ],
        )
        .unwrap();
    registry.observe(&buffer(0), snapshot(1)).unwrap();
    let mut saved = snapshot(1);
    if let Snapshot::Buffer { saved_revision, .. } = &mut saved {
        *saved_revision = Some("r:1".into());
    }
    registry.observe(&buffer(0), saved).unwrap();
    registry
        .observe(
            &job,
            Snapshot::Job {
                state: JobState::Running,
                progress: 10,
            },
        )
        .unwrap();
    registry
        .observe(
            &job,
            Snapshot::Job {
                state: JobState::Cancelling,
                progress: 10,
            },
        )
        .unwrap();
    registry
        .observe(
            &job,
            Snapshot::Job {
                state: JobState::Cancelled,
                progress: 10,
            },
        )
        .unwrap();
    registry
        .observe(
            &Source::Attachment,
            Snapshot::Attachment {
                attached: false,
                generation: "2".into(),
            },
        )
        .unwrap();
    let events = drain(&mut registry);
    assert_eq!(events.len(), 4);
    assert!(
        events
            .iter()
            .all(|delivery| matches!(delivery, Delivery::ReliableChanged(_)))
    );
}

#[test]
fn pair_limit_counts_overlapping_subscriptions_and_failed_reset_preserves_state() {
    let mut registry = Registry::default();
    subscribe(&mut registry, "one", 128);
    subscribe(&mut registry, "two", 128);
    let error = registry
        .subscribe(
            "three".into(),
            vec![buffer(0)],
            vec![(buffer(0), snapshot(0))],
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::LimitExceeded);
    assert_eq!(registry.pairs(), MAX_SOURCES);
    assert_eq!(registry.watched_sources().len(), 128);
    registry.invalidate("one").unwrap();
    assert!(
        registry
            .resync("one", vec![(buffer(0), snapshot(0))])
            .is_err()
    );
    assert!(registry.is_suspended("one"));
    assert_eq!(registry.pairs(), MAX_SOURCES);
    let pending = registry.unsubscribe("one");
    assert_eq!(pending.len(), 1);
    assert!(matches!(&pending[0], Delivery::ResyncRequired(_)));
    assert!(registry.unsubscribe("one").is_empty());
    assert_eq!(registry.pairs(), 128);
    assert_eq!(registry.watched_sources().len(), 128);
}

#[test]
fn wildcard_close_releases_capacity_and_open_is_reliable() {
    let mut registry = Registry::default();
    registry
        .subscribe(
            "sub".into(),
            vec![Source::Buffers],
            (0..MAX_SOURCES)
                .map(|index| (buffer(index), snapshot(0)))
                .collect(),
        )
        .unwrap();
    registry.observe(&buffer(0), Snapshot::Closed {}).unwrap();
    assert_eq!(registry.pairs(), MAX_SOURCES - 1);
    registry
        .discover("sub", buffer(MAX_SOURCES), snapshot(0))
        .unwrap();
    assert_eq!(registry.pairs(), MAX_SOURCES);
    let events = drain(&mut registry);
    assert!(matches!(events[0], Delivery::Closed(_)));
    assert!(matches!(events[1], Delivery::ReliableChanged(_)));
    registry
        .discover("sub", buffer(MAX_SOURCES + 1), snapshot(0))
        .unwrap();
    assert!(registry.is_suspended("sub"));
    assert_eq!(registry.pairs(), MAX_SOURCES);
    assert!(matches!(
        registry.peek_ready(),
        Some(Delivery::ResyncRequired(_))
    ));
}

#[test]
fn reliable_overflow_is_explicit_and_unsubscribe_preserves_queued_lifecycle() {
    let mut registry = Registry::default();
    subscribe(&mut registry, "sub", 1);
    for revision in 1..=MAX_RELIABLE {
        let mut saved = snapshot(revision);
        if let Snapshot::Buffer { saved_revision, .. } = &mut saved {
            *saved_revision = Some(format!("r:{revision}"));
        }
        registry.observe(&buffer(0), saved).unwrap();
    }
    assert_eq!(
        registry
            .observe(&buffer(0), Snapshot::Closed {})
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    let queued = registry.unsubscribe("sub");
    assert_eq!(queued.len(), MAX_RELIABLE);
    assert!(registry.is_empty());
    assert!(registry.watched_sources().is_empty());
    assert!(registry.peek_ready().is_none());
}

#[test]
fn subscription_and_metadata_bounds_are_checked_before_admission() {
    let mut registry = Registry::default();
    for index in 0..MAX_SUBSCRIPTIONS {
        registry
            .subscribe(format!("s:{index}"), vec![Source::Buffers], vec![])
            .unwrap();
    }
    assert_eq!(
        registry
            .subscribe("overflow".into(), vec![Source::Buffers], vec![])
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    registry.unsubscribe("s:0");
    let mut huge = snapshot(0);
    if let Snapshot::Buffer { name, .. } = &mut huge {
        *name = "x".repeat(MAX_STATE_BYTES);
    }
    assert_eq!(
        registry
            .subscribe("huge".into(), vec![buffer(0)], vec![(buffer(0), huge)])
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );
    assert_eq!(registry.len(), MAX_SUBSCRIPTIONS - 1);
    assert!(registry.watched_sources().is_empty());
    assert!(
        registry
            .subscribe(
                "duplicate".into(),
                vec![buffer(0), buffer(0)],
                vec![(buffer(0), snapshot(0))]
            )
            .is_err()
    );
    assert!(
        registry
            .subscribe("missing".into(), vec![buffer(0)], vec![])
            .is_err()
    );
    assert!(
        registry
            .subscribe(
                "control".into(),
                vec![Source::Buffer {
                    buffer: "bad\u{85}".into()
                }],
                vec![]
            )
            .is_err()
    );
}

#[test]
fn reliable_promotion_reports_all_omitted_states_without_leaking_counts() {
    let mut registry = Registry::default();
    subscribe(&mut registry, "sub", 2);
    for revision in 1..=3 {
        registry.observe(&buffer(0), snapshot(revision)).unwrap();
    }
    let mut saved = snapshot(3);
    if let Snapshot::Buffer { saved_revision, .. } = &mut saved {
        *saved_revision = Some("r:3".into());
    }
    registry.observe(&buffer(0), saved).unwrap();
    let Some(Delivery::ReliableChanged(change)) = registry.peek_ready() else {
        panic!()
    };
    assert_eq!(change.coalesced, 3);
    registry.ack_ready();
    registry.observe(&buffer(1), snapshot(1)).unwrap();
    let Some(Delivery::Changed(change)) = registry.peek_ready() else {
        panic!()
    };
    assert_eq!(change.coalesced, 0);
}
