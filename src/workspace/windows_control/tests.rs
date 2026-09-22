// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    protocol::{ClientRequest, HostResponse, VERSION},
    test_support::TestRuntimeRoot,
    workspace::{
        windows_catalog::{
            remember,
            tests::{answer, health, layout, runtime, server},
        },
        windows_transport::{LocalServer, ServerEvent},
    },
};
use std::{path::Path, time::Duration};
use tokio::time::{Instant, timeout_at};

#[test]
fn stopped_name_and_clean_use_one_proven_history_snapshot() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("control-stopped").unwrap();
        let layout = layout(&root, "project", "cache");
        remember(&layout).unwrap();
        let mut controls =
            ControlSnapshot::observe(layout.discovery_scope(), Path::new(".runyte"), false)
                .await
                .unwrap();
        let selected = controls
            .select(UserSelector {
                selector: layout.project_root(),
                working_directory: None,
            })
            .unwrap();
        assert!(!controls.history().entries()[selected].row().running);
        let renamed = controls.rename(selected, "renamed").await.unwrap();
        assert!(renamed.cache_issue.is_none());
        assert!(!controls.recovery_pending());
        assert_eq!(
            controls.clean().unwrap(),
            0,
            "the renamed cache entry changed"
        );

        let current =
            ControlSnapshot::observe(layout.discovery_scope(), Path::new(".runyte"), false)
                .await
                .unwrap();
        let selected = current
            .select(UserSelector {
                selector: Path::new("renamed"),
                working_directory: None,
            })
            .unwrap();
        assert_eq!(current.clean().unwrap(), 1);
        assert!(matches!(
            current.history().target(selected),
            Some(HistoryTarget::Stopped { .. })
        ));
        assert!(
            layout.name_store_root().exists(),
            "clean preserves name storage"
        );
        let empty = ControlSnapshot::observe(layout.discovery_scope(), Path::new(".runyte"), false)
            .await
            .unwrap();
        assert!(empty.history().entries().is_empty());
        assert!(!root.join("project/.runyte/host").exists());
    });
}

#[test]
fn live_rename_targets_the_retained_publication_without_local_fallback() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("control-live-rename").unwrap();
        let layout = layout(&root, "project", "cache");
        let (_endpoint, mut host) = server(&layout, "before");
        let (controls, ()) = tokio::join!(
            ControlSnapshot::observe(layout.discovery_scope(), Path::new(".runyte"), false),
            answer(&mut host, health())
        );
        let mut controls = controls.unwrap();
        let index = controls
            .select(UserSelector {
                selector: Path::new("before"),
                working_directory: None,
            })
            .unwrap();
        let response = async {
            let mut replies = None;
            timeout_at(Instant::now() + Duration::from_secs(5), async {
                loop {
                    match host.recv().await.unwrap() {
                        ServerEvent::Connected { id, responses, .. } => {
                            responses
                                .send(HostResponse::Welcome {
                                    protocol: VERSION,
                                    pid: std::process::id(),
                                    features: vec![
                                        crate::protocol::FeatureGroup::Control,
                                        crate::protocol::FeatureGroup::Buffers,
                                        crate::protocol::FeatureGroup::Wait,
                                    ],
                                    host_version: env!("CARGO_PKG_VERSION").into(),
                                })
                                .await
                                .unwrap();
                            replies = Some((id, responses));
                        }
                        ServerEvent::Request {
                            id,
                            request: ClientRequest::RenameHost { name },
                        } => {
                            assert_eq!(name, "after");
                            let (owner, sender) = replies.as_ref().unwrap();
                            assert_eq!(id, *owner);
                            sender
                                .send(HostResponse::HostRenamed { name })
                                .await
                                .unwrap();
                            break;
                        }
                        ServerEvent::Disconnected { id }
                            if replies.as_ref().map(|(owner, _)| *owner) != Some(id) => {}
                        event => panic!("unexpected control request: {event:?}"),
                    }
                }
            })
            .await
            .unwrap();
        };
        let (result, ()) = tokio::join!(controls.rename(index, "after"), response);
        assert!(result.unwrap().cache_issue.is_none());
        assert!(!root.join("cache/runyte/workspaces.json").exists());
        assert!(!layout.name_store_root().exists());
        host.shutdown().await.unwrap();
    });
}

#[test]
fn failure_text_is_byte_bounded_without_splitting_unicode() {
    let value = "é".repeat(600);
    let summary = bounded(value);
    assert!(summary.len() <= MAX_FAILURE_TEXT_BYTES);
    assert!(summary.ends_with("..."));
}

async fn refuse_stop(host: &mut LocalServer) {
    let mut replies = None;
    timeout_at(Instant::now() + Duration::from_secs(5), async {
        loop {
            match host.recv().await.unwrap() {
                ServerEvent::Connected { id, responses, .. } => {
                    responses
                        .send(HostResponse::Welcome {
                            protocol: VERSION,
                            pid: std::process::id(),
                            features: vec![
                                crate::protocol::FeatureGroup::Control,
                                crate::protocol::FeatureGroup::Buffers,
                                crate::protocol::FeatureGroup::Wait,
                            ],
                            host_version: env!("CARGO_PKG_VERSION").into(),
                        })
                        .await
                        .unwrap();
                    replies = Some((id, responses));
                }
                ServerEvent::Request {
                    id,
                    request: ClientRequest::Shutdown,
                } => {
                    let (owner, sender) = replies.as_ref().unwrap();
                    assert_eq!(id, *owner);
                    sender
                        .send(HostResponse::Refused {
                            message: "protected state remains".into(),
                        })
                        .await
                        .unwrap();
                    return;
                }
                ServerEvent::Disconnected { id }
                    if replies.as_ref().map(|(owner, _)| *owner) != Some(id) => {}
                event => panic!("unexpected control request: {event:?}"),
            }
        }
    })
    .await
    .unwrap();
}

async fn hold_stop_reply(host: &mut LocalServer) {
    let mut replies = None;
    timeout_at(Instant::now() + Duration::from_secs(5), async {
        loop {
            match host.recv().await.unwrap() {
                ServerEvent::Connected { id, responses, .. } => {
                    responses
                        .send(HostResponse::Welcome {
                            protocol: VERSION,
                            pid: std::process::id(),
                            features: vec![
                                crate::protocol::FeatureGroup::Control,
                                crate::protocol::FeatureGroup::Buffers,
                                crate::protocol::FeatureGroup::Wait,
                            ],
                            host_version: env!("CARGO_PKG_VERSION").into(),
                        })
                        .await
                        .unwrap();
                    replies = Some((id, responses));
                }
                ServerEvent::Request {
                    id,
                    request: ClientRequest::Shutdown,
                } => {
                    // The owner has sent its request. Returning without an
                    // answer makes cancellation's unknown outcome observable.
                    assert_eq!(replies.as_ref().map(|(owner, _)| *owner), Some(id));
                    return;
                }
                ServerEvent::Disconnected { id }
                    if replies.as_ref().map(|(owner, _)| *owner) != Some(id) => {}
                event => panic!("unexpected control request: {event:?}"),
            }
        }
    })
    .await
    .unwrap();
}

#[test]
fn stop_all_keeps_distinct_refusals_and_attempts_every_publication() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("control-stop-all").unwrap();
        let first = layout(&root, "first", "cache");
        let second = layout(&root, "second", "cache");
        let (_, mut a) = server(&first, "first");
        let (_, mut b) = server(&second, "second");
        let (controls, (), ()) = tokio::join!(
            ControlSnapshot::observe(first.discovery_scope(), Path::new(".runyte"), false),
            answer(&mut a, health()),
            answer(&mut b, health())
        );
        let controls = controls.unwrap();
        let mut operation = controls.stop_all(false).unwrap();
        let ((), (), ()) = tokio::join!(
            operation.run_to_completion(),
            refuse_stop(&mut a),
            refuse_stop(&mut b)
        );
        let report = operation.report();
        assert_eq!((report.stopped, report.failed, report.total), (0, 2, 2));
        assert_eq!(report.unknown, 0);
        assert_eq!(report.failures.len(), 2);
        assert!(
            report
                .failures
                .iter()
                .all(|failure| failure.contains("protected state remains"))
        );
        assert!(first.cache_root().unwrap().unwrap().join("hosts").exists());
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
    });
}

#[test]
fn cancelled_stop_all_retains_admitted_identity_and_continues_other_hosts() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("control-stop-all-cancel").unwrap();
        let first = layout(&root, "first", "cache");
        let second = layout(&root, "second", "cache");
        let (_, mut a) = server(&first, "first");
        let (_, mut b) = server(&second, "second");
        let (controls, (), ()) = tokio::join!(
            ControlSnapshot::observe(first.discovery_scope(), Path::new(".runyte"), false),
            answer(&mut a, health()),
            answer(&mut b, health())
        );
        let controls = controls.unwrap();
        let mut operation = controls.stop_all(false).unwrap();
        {
            let run = operation.run_to_completion();
            tokio::pin!(run);
            tokio::select! {
                biased;
                _ = hold_stop_reply(&mut a) => {}
                _ = &mut run => panic!("stop-all completed without the held reply"),
            }
        }
        let pending = operation.report();
        assert_eq!(pending.unknown, 1);
        let identity = pending.admitted_summary.unwrap();
        assert!(identity.contains("incarnation="));
        assert!(identity.contains("pipe="));

        let ((), ()) = tokio::join!(operation.run_to_completion(), refuse_stop(&mut b));
        let report = operation.report();
        assert_eq!(
            (report.total, report.stopped, report.failed, report.unknown),
            (2, 0, 1, 1)
        );
        assert!(report.admitted_summary.is_none());
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.contains("outcome unknown"))
        );
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
    });
}
