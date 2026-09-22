// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    private_storage::Directory,
    protocol::{ClientRequest, FeatureGroup, HostResponse, SessionPreview, VERSION},
    test_support::TestRuntimeRoot,
    workspace::windows_location::{CapturedRoots, DiscoveryInputs},
    workspace::{
        recent_history::{RecentEntry, encode_recents},
        windows_endpoint::{EndpointLocation, NameStore},
        windows_location::{LocationInputs, ResolvedLayout},
        windows_transport::{LocalServer, ServerEvent},
    },
};
use std::{ffi::OsStr, fs};

fn layout(root: &TestRuntimeRoot, project: &str, namespace: &str) -> ResolvedLayout {
    let project = root.join(project);
    fs::create_dir_all(&project).unwrap();
    ResolvedLayout::resolve(LocationInputs {
        state_root: project.join(".runyte"),
        project_root: project,
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            cache_home: Some(root.join(namespace)),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    })
    .unwrap()
}

fn server(layout: &ResolvedLayout, name: &str) -> (EndpointLocation, LocalServer) {
    let endpoint = layout.publication_location().unwrap();
    let server = LocalServer::bind(endpoint.prepare(Some(name.to_owned())).unwrap()).unwrap();
    (endpoint, server)
}

fn named_server(layout: &ResolvedLayout, name: &str) -> (EndpointLocation, LocalServer) {
    let endpoint = layout.publication_location().unwrap();
    let names = NameStore::open(layout.state_root()).unwrap();
    let server = LocalServer::bind_with_names(
        endpoint
            .prepare_named(&names, Some(name.to_owned()))
            .unwrap(),
        names,
    )
    .unwrap();
    (endpoint, server)
}

fn rewrite_endpoint_protocols_below(path: &Path, protocol: u32) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rewrite_endpoint_protocols_below(&path, protocol);
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        if let Ok(mut metadata) =
            crate::workspace::windows_endpoint::EndpointMetadata::from_json(&bytes)
        {
            metadata.protocol = protocol;
            fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
            continue;
        }
        if let Ok(mut record) =
            crate::workspace::windows_endpoint::RegistryRecord::from_json(&bytes)
        {
            record.host.protocol = protocol;
            fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        }
    }
}

fn health() -> HostResponse {
    HostResponse::Health {
        protocol: VERSION,
        pid: std::process::id(),
        interactive_attached: false,
        unsaved_buffers: 0,
        open_buffers: 0,
        pending_wait_requests: 0,
        plugin_jobs: 0,
        activity_leases: 0,
        activities: Box::new([]),
        live_terminals: 0,
        terminal_sessions: 0,
        terminal_line_activity_unix_seconds: None,
        unread_terminals: 0,
        terminal_bell: false,
    }
}

async fn answer(server: &mut LocalServer, health: HostResponse) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut responses = None;
        loop {
            match server.recv().await.unwrap() {
                ServerEvent::Connected {
                    responses: sender, ..
                } => {
                    sender
                        .send(HostResponse::Welcome {
                            protocol: VERSION,
                            pid: std::process::id(),
                            features: vec![
                                FeatureGroup::Control,
                                FeatureGroup::Buffers,
                                FeatureGroup::Wait,
                            ],
                            host_version: env!("CARGO_PKG_VERSION").to_owned(),
                        })
                        .await
                        .unwrap();
                    responses = Some(sender);
                }
                ServerEvent::Request {
                    request: ClientRequest::Health,
                    ..
                } => {
                    responses.as_ref().unwrap().send(health).await.unwrap();
                    return;
                }
                ServerEvent::Disconnected { .. } => {}
                other => panic!("unexpected health fixture event: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
}

fn fixture(name: &str) -> (TestRuntimeRoot, DiscoveryScope) {
    let root = TestRuntimeRoot::new(name).unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots: Vec::new(),
        roots: CapturedRoots {
            runtime_root: Some(runtime),
            cache_home: None,
            local_app_data: None,
            inventory_override: None,
        },
    })
    .unwrap();
    (root, scope)
}

async fn answer_preview(server: &mut LocalServer) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut responses = None;
        loop {
            match server.recv().await.unwrap() {
                ServerEvent::Connected {
                    responses: sender, ..
                } => {
                    sender
                        .send(HostResponse::Welcome {
                            protocol: VERSION,
                            pid: std::process::id(),
                            features: vec![
                                FeatureGroup::Control,
                                FeatureGroup::Buffers,
                                FeatureGroup::Wait,
                            ],
                            host_version: env!("CARGO_PKG_VERSION").to_owned(),
                        })
                        .await
                        .unwrap();
                    responses = Some(sender);
                }
                ServerEvent::Request {
                    request: ClientRequest::SessionPreview,
                    ..
                } => {
                    responses
                        .as_ref()
                        .unwrap()
                        .send(HostResponse::SessionPreview {
                            preview: SessionPreview {
                                layout_panes: 1,
                                panes: Vec::new(),
                                omitted_panes: 0,
                                other_resources: Vec::new(),
                                omitted_resources: 0,
                            },
                        })
                        .await
                        .unwrap();
                    return;
                }
                ServerEvent::Disconnected { .. } => {}
                other => panic!("unexpected preview fixture event: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn retained_live_selection_previews_exact_peer_and_replacement_gets_new_key() {
    let root = TestRuntimeRoot::new("native-service-replacement").unwrap();
    let layout = layout(&root, "project", "cache");
    let (_, mut old) = server(&layout, "old");
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(1, false).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        },
        answer(&mut old, health())
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = event
    else {
        panic!("first complete refresh failed")
    };
    assert_eq!(rows.len(), 1);
    let old_selection = rows[0].selection();
    assert!(old_selection.publication_key().is_some());

    let (prepared, ()) = tokio::join!(
        handle.prepare_selected_live(old_selection.clone()),
        answer(&mut old, health())
    );
    let prepared = prepared.unwrap();
    assert_eq!(prepared.metadata().address, old.metadata().address);
    assert_eq!(prepared.peer().identity(), prepared.metadata().process);
    assert!(prepared.peer().is_alive().unwrap());

    let (event, ()) = tokio::join!(
        async {
            handle.try_preview(2, old_selection.clone()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        },
        answer_preview(&mut old)
    );
    assert!(matches!(
        event,
        WorkspaceEvent::Previewed {
            generation: 2,
            selection,
            result: Ok(preview),
            ..
        } if selection == old_selection && preview.layout_panes == 1
    ));
    old.shutdown().await.unwrap();
    let (_, mut replacement) = server(&layout, "replacement");
    let (stale, ()) = tokio::join!(
        handle.prepare_selected_live(old_selection.clone()),
        answer(&mut replacement, health())
    );
    assert!(stale.unwrap_err().to_string().contains("choose it again"));
    handle.try_preview(3, old_selection.clone()).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Previewed {
            generation: 3,
            selection,
            result: Err(_),
            ..
        } if selection == old_selection
    ));
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(4, false).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        },
        answer(&mut replacement, health())
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(replacement_rows),
        ..
    } = event
    else {
        panic!("replacement refresh failed")
    };
    assert_eq!(replacement_rows.len(), 1);
    assert_ne!(replacement_rows[0].selection(), old_selection);
    handle
        .try_rename_selected(5, old_selection, "misdirected")
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Renamed {
            generation: 5,
            result: Err(error),
            ..
        } if error.contains("choose it again")
    ));
    owner.shutdown().await.unwrap();
    replacement.shutdown().await.unwrap();
}

#[tokio::test]
async fn prepared_live_selection_keeps_its_key_across_verified_rename() {
    let root = TestRuntimeRoot::new("native-service-prepared-rename").unwrap();
    let layout = layout(&root, "project", "cache");
    let (_, mut host) = named_server(&layout, "before");
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(1, false).unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut host, health())
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = event
    else {
        panic!("rename preparation setup failed")
    };
    let selection = rows[0].selection();
    host.try_rename("after").unwrap().wait().await.unwrap();
    let (prepared, ()) = tokio::join!(
        handle.prepare_selected_live(selection.clone()),
        answer(&mut host, health())
    );
    let prepared = prepared.unwrap();
    assert_eq!(prepared.metadata().name.as_deref(), Some("after"));
    assert_eq!(
        crate::workspace::PublicationKey::from_authenticated_metadata(prepared.metadata()),
        selection.publication_key().unwrap()
    );
    owner.shutdown().await.unwrap();
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn prepared_selection_uses_exact_key_for_same_project_hidden_publications() {
    let root = TestRuntimeRoot::new("native-service-prepare-exact-key").unwrap();
    let visible = layout(&root, "project", "visible-cache");
    let hidden = ResolvedLayout::resolve(LocationInputs {
        project_root: visible.project_root().to_owned(),
        state_root: root.join("hidden-state"),
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            cache_home: Some(root.join("hidden-cache")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    })
    .unwrap();
    let (_, mut visible_host) = server(&visible, "visible");
    let (_, mut hidden_host) = server(&hidden, "hidden");
    let hidden_address = hidden_host.metadata().address.clone();
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        visible.discovery_scope().clone(),
        Some(visible.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    let (event, (), ()) = tokio::join!(
        async {
            handle.try_refresh(1, true).unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut visible_host, health()),
        answer(&mut hidden_host, health())
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = event
    else {
        panic!("hidden exact-key setup failed")
    };
    assert_eq!(rows.len(), 2);
    let selection = rows
        .iter()
        .find(|row| row.name.as_deref() == Some("hidden"))
        .unwrap()
        .selection();
    let (prepared, (), ()) = tokio::join!(
        handle.prepare_selected_live(selection.clone()),
        answer(&mut visible_host, health()),
        answer(&mut hidden_host, health())
    );
    assert_eq!(prepared.unwrap().metadata().address, hidden_address);
    let incomplete = tokio::time::timeout(
        Duration::from_secs(5),
        handle.prepare_selected_live(selection),
    )
    .await
    .expect("incomplete preparation exceeded its fixed budget");
    assert!(
        incomplete.is_err(),
        "an incomplete catalog prepared a target"
    );
    owner.shutdown().await.unwrap();
    visible_host.shutdown().await.unwrap();
    hidden_host.shutdown().await.unwrap();
}

#[tokio::test]
async fn project_only_selection_cannot_prepare_or_create_a_live_target() {
    let (root, scope) = fixture("native-service-prepare-stopped");
    let (handle, mut owner, _events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    let project = root.join("stopped-project");
    let error = handle
        .prepare_selected_live(WorkspaceSelection::project_only(project.clone()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no live publication"));
    assert!(!project.exists());
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn exact_incompatible_publication_is_refused_without_a_control_handshake() {
    let root = TestRuntimeRoot::new("native-service-prepare-incompatible").unwrap();
    let layout = layout(&root, "project", "cache");
    let (endpoint, mut host) = server(&layout, "current");
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(1, false).unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut host, health())
    );
    let WorkspaceEvent::Refreshed { result: Ok(_), .. } = event else {
        panic!("incompatible preparation setup failed")
    };
    rewrite_endpoint_protocols_below(root.path(), VERSION + 1);
    let metadata = endpoint.read_ready().unwrap().unwrap();
    assert_eq!(metadata.protocol, VERSION + 1);
    let selection = WorkspaceSelection::selected(
        metadata.project_root().unwrap(),
        crate::workspace::PublicationKey::from_authenticated_metadata(&metadata),
    );
    let error = handle.prepare_selected_live(selection).await.unwrap_err();
    let detail = error.to_string();
    assert!(detail.contains("incompatible session"), "{detail}");
    let handshake = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            match host.recv().await {
                Some(event @ (ServerEvent::Connected { .. } | ServerEvent::Request { .. })) => {
                    return Some(event);
                }
                Some(ServerEvent::Disconnected { .. } | ServerEvent::TransportFailure { .. }) => {}
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await;
    assert!(
        handshake.is_err() || matches!(handshake, Ok(None)),
        "incompatible preparation started a control handshake: {handshake:?}"
    );
    owner.shutdown().await.unwrap();
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_cancels_a_held_native_preview_before_its_io_deadline() {
    let root = TestRuntimeRoot::new("native-service-held-preview").unwrap();
    let layout = layout(&root, "project", "cache");
    let (_, mut host) = server(&layout, "held");
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(1, false).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        },
        answer(&mut host, health())
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = event
    else {
        panic!("held-preview setup did not observe its host")
    };
    let selection = rows[0].selection();
    let (requested, requested_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let ((), ()) = tokio::join!(
        async {
            handle.try_preview(2, selection).unwrap();
            requested_rx.await.unwrap();
            tokio::time::timeout(Duration::from_secs(1), owner.shutdown())
                .await
                .expect("shutdown waited for the preview's two-second IO budget")
                .unwrap();
            release.send(()).unwrap();
        },
        async {
            let mut responses = None;
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match host.recv().await.unwrap() {
                        ServerEvent::Connected {
                            responses: sender, ..
                        } => {
                            sender
                                .send(HostResponse::Welcome {
                                    protocol: VERSION,
                                    pid: std::process::id(),
                                    features: vec![
                                        FeatureGroup::Control,
                                        FeatureGroup::Buffers,
                                        FeatureGroup::Wait,
                                    ],
                                    host_version: env!("CARGO_PKG_VERSION").to_owned(),
                                })
                                .await
                                .unwrap();
                            responses = Some(sender);
                        }
                        ServerEvent::Request {
                            request: ClientRequest::SessionPreview,
                            ..
                        } => {
                            assert!(responses.is_some());
                            requested.send(()).unwrap();
                            let _ = release_rx.await;
                            return;
                        }
                        ServerEvent::Disconnected { .. } => {}
                        other => panic!("unexpected held-preview fixture event: {other:?}"),
                    }
                }
            })
            .await
            .unwrap();
        }
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_preparation_releases_a_held_health_probe_promptly() {
    let root = TestRuntimeRoot::new("native-service-cancel-prepare").unwrap();
    let layout = layout(&root, "project", "cache");
    let (_, mut host) = server(&layout, "held");
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(1, false).unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut host, health())
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = event
    else {
        panic!("held preparation setup failed")
    };
    let selection = rows[0].selection();
    let (seen, seen_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let ((), ()) = tokio::join!(
        async {
            let preparing = tokio::spawn({
                let handle = handle.clone();
                async move { handle.prepare_selected_live(selection).await }
            });
            seen_rx.await.unwrap();
            preparing.abort();
            let _ = preparing.await;
            let (entered, entered_rx) = oneshot::channel();
            handle
                .submit(Request::Emit {
                    generation: 2,
                    entered: Some(entered),
                })
                .unwrap();
            tokio::time::timeout(Duration::from_millis(250), entered_rx)
                .await
                .expect("cancelled preparation retained the worker's health probe")
                .unwrap();
            release.send(()).unwrap();
        },
        async {
            let mut responses = None;
            loop {
                match host.recv().await.unwrap() {
                    ServerEvent::Connected {
                        responses: sender, ..
                    } => {
                        sender
                            .send(HostResponse::Welcome {
                                protocol: VERSION,
                                pid: std::process::id(),
                                features: vec![
                                    FeatureGroup::Control,
                                    FeatureGroup::Buffers,
                                    FeatureGroup::Wait,
                                ],
                                host_version: env!("CARGO_PKG_VERSION").to_owned(),
                            })
                            .await
                            .unwrap();
                        responses = Some(sender);
                    }
                    ServerEvent::Request {
                        request: ClientRequest::Health,
                        ..
                    } => {
                        assert!(responses.is_some());
                        seen.send(()).unwrap();
                        let _ = release_rx.await;
                        break;
                    }
                    ServerEvent::Disconnected { .. } => {}
                    other => panic!("unexpected held preparation event: {other:?}"),
                }
            }
        }
    );
    let _ = events.recv().await;
    owner.shutdown().await.unwrap();
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_refresh_retains_actual_stopped_name_recovery_until_owned_shutdown() {
    let root = TestRuntimeRoot::new("native-service-name-recovery").unwrap();
    root.create_private_dir("project").unwrap();
    let layout = layout(&root, "project", "cache");
    let history_path = layout
        .cache_root()
        .unwrap()
        .unwrap()
        .join("workspaces.json");
    Directory::open(history_path.parent().unwrap(), true)
        .unwrap()
        .atomic_write(
            OsStr::new("workspaces.json"),
            &encode_recents(&[RecentEntry::new(
                layout.project_root().to_owned(),
                Some("old".to_owned()),
                None,
                None,
            )])
            .unwrap(),
        )
        .unwrap();
    let mut snapshot = ControlSnapshot::observe_at(
        layout.discovery_scope(),
        Some(&layout.read_location()),
        Path::new(".runyte"),
        false,
    )
    .await
    .unwrap();
    assert_eq!(snapshot.history().entries().len(), 1);
    let mut edit = snapshot.history().stopped_name_editor(0).unwrap();
    assert!(edit.fault_pending_recovery_for_test("new").is_err());
    assert!(edit.recovery_pending());
    snapshot.retain_pending_name_for_test(edit);
    let names = NameStore::open(layout.state_root()).unwrap();
    let publication = layout
        .publication_location()
        .unwrap()
        .prepare_named(&names, None)
        .unwrap()
        .publish()
        .unwrap();
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn_with_snapshot(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        Some(snapshot),
    )
    .unwrap();
    handle.try_refresh(1, false).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Refreshed {
            generation: 1,
            result: Err(_),
        }
    ));
    drop(publication);
    owner.shutdown().await.unwrap();
    assert_eq!(
        names
            .load(&crate::workspace::workspace_id(layout.project_root()))
            .unwrap(),
        None
    );
    assert_eq!(
        fs::read(history_path).unwrap(),
        encode_recents(&[RecentEntry::new(
            layout.project_root().to_owned(),
            Some("old".to_owned()),
            None,
            None,
        )])
        .unwrap()
    );
}

#[tokio::test]
async fn idle_worker_joins_and_complete_empty_refresh_returns_rows() {
    let (_root, scope) = fixture("native-service-idle");
    let (handle, mut owner, mut events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    handle.try_refresh(9, false).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Refreshed {
            generation: 9,
            result: Ok(rows)
        } if rows.is_empty()
    ));
    tokio::time::timeout(Duration::from_secs(3), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(owner.thread.is_none());
    assert_eq!(
        handle.try_poll(),
        Err("native session service is shutting down")
    );
}

#[tokio::test]
async fn clean_from_fresh_service_observes_complete_catalog_before_mutating_history() {
    let root = TestRuntimeRoot::new("native-service-clean-fresh").unwrap();
    root.create_private_dir("project").unwrap();
    let layout = layout(&root, "project", "cache");
    let history_path = layout
        .cache_root()
        .unwrap()
        .unwrap()
        .join("workspaces.json");
    Directory::open(history_path.parent().unwrap(), true)
        .unwrap()
        .atomic_write(
            OsStr::new("workspaces.json"),
            &encode_recents(&[RecentEntry::new(
                layout.project_root().to_owned(),
                Some("stopped".to_owned()),
                None,
                None,
            )])
            .unwrap(),
        )
        .unwrap();
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    handle.try_clean(7).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Cleaned {
            generation: 7,
            result: Ok(1)
        }
    ));
    handle.try_refresh(8, false).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, WorkspaceEvent::Refreshed { generation: 8, result: Ok(rows) } if rows.is_empty())
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn request_admission_and_event_backpressure_stay_bounded_on_shutdown() {
    let (root, scope) = fixture("native-service-backpressure");
    let (handle, mut owner, events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    let (release, hold) = oneshot::channel();
    let (entered, entered_rx) = oneshot::channel();
    handle
        .submit(Request::Hold {
            entered,
            release: hold,
        })
        .unwrap();
    entered_rx.await.unwrap();
    for generation in 0..REQUEST_CAPACITY {
        handle
            .submit(Request::Emit {
                generation: generation as u64,
                entered: None,
            })
            .unwrap();
    }
    assert_eq!(
        handle.submit(Request::Emit {
            generation: 99,
            entered: None
        }),
        Err("native session service queue is full")
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while events.len() < EVENT_CAPACITY {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (entered, entered_rx) = oneshot::channel();
    handle.submit(Request::EmitWarnings { entered }).unwrap();
    entered_rx.await.unwrap();
    let selection = WorkspaceSelection::selected(
        root.join("blocked-project"),
        crate::workspace::PublicationKey::for_test(b"blocked-publication"),
    );
    let started = std::time::Instant::now();
    let error = handle
        .prepare_selected_live(selection.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("preparation timed out"));
    assert!(started.elapsed() < Duration::from_secs(5));

    let preparing = handle.prepare_selected_live(selection);
    tokio::pin!(preparing);
    tokio::select! {
        biased;
        result = &mut preparing => panic!("blocked preparation completed before shutdown: {result:?}"),
        _ = std::future::ready(()) => {}
    }
    let (shutdown, preparing) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(1), owner.shutdown()),
        tokio::time::timeout(Duration::from_secs(1), &mut preparing)
    );
    shutdown.unwrap().unwrap();
    let error = preparing.unwrap().unwrap_err();
    assert!(error.to_string().contains("shutting down"));
    assert!(owner.thread.is_none());
}

#[tokio::test]
async fn cancelled_shutdown_keeps_completion_and_join_for_retry() {
    let (_root, scope) = fixture("native-service-cancelled-shutdown");
    let (handle, mut owner, _events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    let (release, hold) = oneshot::channel();
    let (entered, entered_rx) = oneshot::channel();
    handle
        .submit(Request::Hold {
            entered,
            release: hold,
        })
        .unwrap();
    entered_rx.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), owner.shutdown())
            .await
            .is_err()
    );
    assert!(owner.completed.is_some());
    assert!(owner.thread.is_some());
    assert_eq!(
        handle.try_poll(),
        Err("native session service is shutting down")
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(owner.thread.is_none());
}

#[tokio::test]
async fn latest_preview_and_stale_selection_do_not_resolve_project_path() {
    let (root, scope) = fixture("native-service-selection");
    let (handle, mut owner, mut events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    handle.try_refresh(1, false).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap();
    let selection = WorkspaceSelection::project_only(root.join("unseen-project"));
    let (release, hold) = oneshot::channel();
    let (entered, entered_rx) = oneshot::channel();
    handle
        .submit(Request::Hold {
            entered,
            release: hold,
        })
        .unwrap();
    entered_rx.await.unwrap();
    for generation in 2..20 {
        handle.try_preview(generation, selection.clone()).unwrap();
    }
    release.send(()).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Previewed {
            generation: 19,
            result: Err(_),
            ..
        }
    ));
    handle
        .try_stop_selected(20, selection.clone(), true)
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Stopped {
            generation: 20,
            selection: Some(returned),
            result: Err(error),
            ..
        } if returned == selection && error.contains("choose it again")
    ));
    owner.shutdown().await.unwrap();
    assert!(!root.join("unseen-project").exists());
}

#[tokio::test]
async fn closed_event_receiver_and_worker_panic_are_joined() {
    let (_root, scope) = fixture("native-service-receiver-closed");
    let (handle, mut owner, events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    drop(events);
    let (entered, entered_rx) = oneshot::channel();
    handle
        .submit(Request::Emit {
            generation: 1,
            entered: Some(entered),
        })
        .unwrap();
    entered_rx.await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !handle.requests.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let error = owner.shutdown().await.unwrap_err();
    assert!(error.to_string().contains("event receiver closed"));
    assert!(owner.thread.is_none());

    let (_root, scope) = fixture("native-service-panic");
    let (handle, mut owner, _events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    let (entered, entered_rx) = oneshot::channel();
    handle.submit(Request::Panic { entered }).unwrap();
    entered_rx.await.unwrap();
    let error = owner.shutdown().await.unwrap_err();
    assert!(error.to_string().contains("panicked"));
    assert!(owner.thread.is_none());
}

#[test]
fn request_sizes_are_checked_before_owned_queue_admission() {
    let (root, scope) = fixture("native-service-bounds");
    let (handle, owner, _events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();
    let huge = PathBuf::from("x".repeat(MAX_PERSISTED_PATH_BYTES + 1));
    assert_eq!(
        handle.try_stop_selector(1, &huge, None, false),
        Err("native session path is too long")
    );
    assert_eq!(
        handle.try_rename_selector(1, Path::new("somewhere"), None, &"n".repeat(65)),
        Err("session name is too long")
    );
    assert_eq!(
        handle.try_directory_worktrees(1, Path::new("relative")),
        Err("worktree discovery requires an absolute project path")
    );
    assert_eq!(
        discover_worktrees(None, &root.join("missing-git")),
        Ok(Vec::new())
    );
    // Drop joins the idle worker even without an explicit async shutdown.
    drop(owner);
}
