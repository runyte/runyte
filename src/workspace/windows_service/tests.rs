// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    private_storage::Directory,
    protocol::{
        ClientRequest, FeatureGroup, HostResponse, OpenDestination, OpenDestinationEntry,
        SessionPreview, VERSION,
    },
    test_support::TestRuntimeRoot,
    workspace::windows_location::{CapturedRoots, DiscoveryInputs},
    workspace::{
        recent_history::{RecentEntry, encode_recents},
        windows_endpoint::{EndpointLocation, NameStore, RegistrySet},
        windows_location::{LocationInputs, ResolvedLayout},
        windows_transport::{LocalServer, ServerEvent},
    },
};
use std::{
    ffi::OsStr,
    fs,
    io::Read,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::Command,
};
use windows_sys::Win32::System::{
    JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
    Threading::{CREATE_NO_WINDOW, GetCurrentProcess},
};

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

async fn receive_while_answering_health(
    server: &mut LocalServer,
    events: &mut mpsc::Receiver<WorkspaceEvent>,
) -> WorkspaceEvent {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut responses = None;
        loop {
            tokio::select! {
                biased;
                event = events.recv() => return event.expect("workspace service event"),
                event = server.recv() => match event.expect("health fixture server event") {
                    ServerEvent::Connected { responses: sender, .. } => {
                        sender.send(HostResponse::Welcome {
                            protocol: VERSION,
                            pid: std::process::id(),
                            features: vec![FeatureGroup::Control, FeatureGroup::Buffers, FeatureGroup::Wait],
                            host_version: env!("CARGO_PKG_VERSION").to_owned(),
                        }).await.unwrap();
                        responses = Some(sender);
                    }
                    ServerEvent::Request { request: ClientRequest::Health, .. } => {
                        responses.as_ref().expect("connected health client").send(health()).await.unwrap();
                    }
                    ServerEvent::Disconnected { .. } => {}
                    other => panic!("unexpected health fixture event: {other:?}"),
                },
            }
        }
    })
    .await
    .expect("workspace service health observation")
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

async fn answer_inventory(server: &mut LocalServer) {
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
                    request: ClientRequest::DestinationInventory,
                    ..
                } => {
                    responses
                        .as_ref()
                        .unwrap()
                        .send(HostResponse::DestinationInventory {
                            incarnation: "a".repeat(64),
                            entries: vec![OpenDestinationEntry {
                                destination: OpenDestination::Buffer(1),
                                label: "note.txt".to_owned(),
                                detail: "buffer".to_owned(),
                            }],
                            truncated: false,
                        })
                        .await
                        .unwrap();
                    return;
                }
                ServerEvent::Disconnected { .. } => {}
                other => panic!("unexpected inventory fixture event: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn selected_native_inventory_returns_exact_live_host_destination_identity() {
    let root = TestRuntimeRoot::new("native-service-inventory").unwrap();
    let layout = layout(&root, "project", "cache");
    let (_, mut host) = server(&layout, "inventory");
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
        panic!("inventory setup did not observe its host")
    };
    let selection = rows[0].selection();
    assert!(selection.publication_key().is_some());
    assert!(
        handle
            .try_inventory(
                2,
                WorkspaceSelection::project_only(layout.project_root().to_owned())
            )
            .is_err()
    );
    let (event, ()) = tokio::join!(
        async {
            handle.try_inventory(3, selection.clone()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        },
        async {
            answer(&mut host, health()).await;
            answer_inventory(&mut host).await;
        }
    );
    assert!(matches!(
        event,
        WorkspaceEvent::Inventory {
            generation: 3,
            selection: received,
            result: Ok(inventory),
            ..
        } if received == selection
            && inventory.incarnation == "a".repeat(64)
            && inventory.entries.len() == 1
            && inventory.entries[0].destination == OpenDestination::Buffer(1)
    ));
    owner.shutdown().await.unwrap();
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn selected_native_number_mutates_only_the_fresh_exact_live_publication() {
    let root = TestRuntimeRoot::new("native-service-selected-number").unwrap();
    let layout = layout(&root, "project", "cache");
    let history_path = layout
        .cache_root()
        .unwrap()
        .unwrap()
        .join("workspaces.json");
    assert!(
        !history_path.exists(),
        "the host starts without seeded history"
    );
    let (_, mut old) = server(&layout, "old");
    let startup =
        ParentAttachStartup::capture(&root.join("missing-startup.exe"), None, 0, None).unwrap();
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn_with_current_layout(
        layout.clone(),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();
    handle.try_ensure_current_record().unwrap();
    handle.try_record_current_activity(42).unwrap();
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(1, false).unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut old, health())
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = event
    else {
        panic!("number setup did not observe its host")
    };
    let old_selection = rows[0].selection();
    assert!(
        handle
            .try_number_selected(
                2,
                WorkspaceSelection::project_only(layout.project_root().to_owned()),
                Some(4)
            )
            .is_err()
    );
    let (event, ()) = tokio::join!(
        async {
            handle
                .try_number_selected(3, old_selection.clone(), Some(4))
                .unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut old, health())
    );
    assert!(matches!(
        event,
        WorkspaceEvent::Numbered {
            generation: 3,
            selection: Some(selection),
            number: Some(4),
            result: Ok(None),
            ..
        } if selection == old_selection
    ));
    old.shutdown().await.unwrap();
    handle.try_refresh(4, false).unwrap();
    let event = events.recv().await.unwrap();
    assert!(
        matches!(
            &event,
            WorkspaceEvent::Refreshed {
                generation: 4,
                result: Ok(rows),
            } if rows.len() == 1
                && !rows[0].running
                && rows[0].number.is_none()
                && rows[0].last_active_unix_seconds == Some(42)
        ),
        "stopped row after host closure: {event:?}"
    );
    let remembered = crate::workspace::recent_history::read_recents(Some(&history_path)).unwrap();
    assert_eq!(remembered.len(), 1);
    assert_eq!(remembered[0].project_root, layout.project_root());
    assert_eq!(remembered[0].number, None);
    assert!(!remembered[0].number_pinned);
    let (_, mut replacement) = server(&layout, "replacement");
    handle
        .try_number_selected(5, old_selection.clone(), Some(5))
        .unwrap();
    let event = receive_while_answering_health(&mut replacement, &mut events).await;
    assert!(
        matches!(
            &event,
            WorkspaceEvent::Numbered {
                generation: 5,
                selection: Some(selection),
                result: Err(error),
                ..
            } if *selection == old_selection && error.contains("choose it again")
        ),
        "stale number selection outcome: {event:?}"
    );
    handle.try_refresh(6, false).unwrap();
    let event = receive_while_answering_health(&mut replacement, &mut events).await;
    assert!(matches!(
        event,
        WorkspaceEvent::Refreshed {
            generation: 6,
            result: Ok(rows),
        } if rows.len() == 1 && rows[0].number == Some(1)
    ));
    owner.shutdown().await.unwrap();
    replacement.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_number_compacts_stopped_digit_before_explicit_live_renumber() {
    let root = TestRuntimeRoot::new("native-number-stopped-compaction").unwrap();
    let stopped = layout(&root, "stopped", "cache");
    let running = layout(&root, "running", "cache");
    let history_path = running
        .cache_root()
        .unwrap()
        .unwrap()
        .join("workspaces.json");
    Directory::open(history_path.parent().unwrap(), true)
        .unwrap()
        .atomic_write(
            OsStr::new("workspaces.json"),
            &encode_recents(&[
                RecentEntry::new(stopped.project_root().to_owned(), None, Some(1), None),
                RecentEntry::new(running.project_root().to_owned(), None, Some(2), None),
            ])
            .unwrap(),
        )
        .unwrap();
    let (_, mut host) = server(&running, "running");
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        running.discovery_scope().clone(),
        Some(running.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    let (event, ()) = tokio::join!(
        async {
            handle.try_refresh(1, false).unwrap();
            events.recv().await.unwrap()
        },
        async {
            answer(&mut host, health()).await;
        }
    );
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = event
    else {
        panic!("number compaction did not return complete rows");
    };
    let live = rows
        .iter()
        .find(|row| row.project_root == running.project_root())
        .unwrap();
    assert_eq!(live.number, Some(1));
    let selection = live.selection();
    let stopped_row = rows
        .iter()
        .find(|row| row.project_root == stopped.project_root())
        .unwrap();
    assert!(!stopped_row.running);
    assert_eq!(stopped_row.number, None);
    let remembered = crate::workspace::recent_history::read_recents(Some(&history_path)).unwrap();
    assert_eq!(remembered[0].number, None);
    assert_eq!(remembered[1].number, Some(1));

    let (event, ()) = tokio::join!(
        async {
            handle
                .try_number_selected(2, selection.clone(), Some(1))
                .unwrap();
            events.recv().await.unwrap()
        },
        async {
            answer(&mut host, health()).await;
        }
    );
    assert!(matches!(
        event,
        WorkspaceEvent::Numbered {
            generation: 2,
            selection: Some(received),
            result: Ok(None),
            ..
        } if received == selection
    ));
    owner.shutdown().await.unwrap();
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn selected_native_forget_removes_only_an_unchanged_stopped_history_row() {
    let root = TestRuntimeRoot::new("native-service-selected-forget").unwrap();
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
    let stopped = WorkspaceSelection::project_only(layout.project_root().to_owned());
    handle.try_forget_selected(6, stopped.clone()).unwrap();
    let event = events.recv().await.unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Forgotten {
            generation: 6,
            result: Ok(true),
            ..
        }
    ));
    assert!(
        handle
            .try_forget_selected(
                7,
                WorkspaceSelection::selected(
                    layout.project_root().to_owned(),
                    crate::workspace::PublicationKey::for_test(b"live"),
                ),
            )
            .is_err()
    );
    handle.try_refresh(8, false).unwrap();
    let event = events.recv().await.unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Refreshed {
            generation: 8,
            result: Ok(rows),
        } if rows.is_empty()
    ));
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn worktree_teardown_forgets_captured_record_after_directory_is_removed() {
    let root = TestRuntimeRoot::new("native-service-worktree-teardown").unwrap();
    let layout = layout(&root, "project", "cache");
    let project = layout.project_root().to_owned();
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
                project.clone(),
                Some("worktree".to_owned()),
                None,
                None,
            )])
            .unwrap(),
        )
        .unwrap();
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        None,
        PathBuf::from(".runyte"),
    )
    .unwrap();
    handle
        .try_prepare_worktree_teardown(31, &project, None)
        .unwrap();
    let WorkspaceEvent::WorktreePrepared {
        generation: 31,
        path,
        result,
    } = events.recv().await.unwrap()
    else {
        panic!("worktree prepare did not return its lease");
    };
    let prepared = (*result).unwrap();
    assert_eq!(path, project);
    assert!(prepared.stopped_session().is_none());
    let lease = prepared.into_lease();

    handle
        .try_finish_worktree_teardown(31, &project, lease.clone())
        .unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        WorkspaceEvent::WorktreeFinalized {
            generation: 31,
            result: Err(error),
            ..
        } if error.contains("still exists")
    ));
    fs::remove_dir(&project).unwrap();
    handle
        .try_finish_worktree_teardown(31, &project, lease)
        .unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        WorkspaceEvent::WorktreeFinalized {
            generation: 31,
            result: Ok(true),
            ..
        }
    ));
    assert!(
        crate::workspace::recent_history::read_recents(Some(&history_path))
            .unwrap()
            .is_empty()
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn worktree_inspection_finds_a_ready_only_host_and_refuses_unreviewed_stop() {
    let root = TestRuntimeRoot::new("native-service-worktree-ready-only").unwrap();
    let layout = layout(&root, "project", "source-cache");
    let project = layout.project_root().to_owned();
    let other_cache = root.create_private_dir("other-cache").unwrap();
    let other_inventory = root.create_private_dir("other-inventory").unwrap();
    // A complete empty scope needs the same stable registry identities as a
    // populated one. The host below publishes only to the source scope.
    let _empty_scope =
        RegistrySet::with_inventory(&[other_cache.join("hosts")], Some(other_inventory.clone()))
            .unwrap();
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            cache_home: Some(other_cache),
            inventory_override: Some(other_inventory),
            ..CapturedRoots::default()
        },
    })
    .unwrap();
    let (handle, mut owner, mut events) =
        WorkspaceServiceOwner::spawn(scope, None, PathBuf::from(".runyte")).unwrap();

    handle.try_refresh(1, true).unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        WorkspaceEvent::Refreshed {
            result: Ok(rows),
            ..
        } if rows.is_empty()
    ));
    handle.try_inspect_worktree_teardown(2, &project).unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        WorkspaceEvent::WorktreeInspected {
            generation: 2,
            result,
            ..
        } if matches!(*result, Ok(None))
    ));
    let (_, mut host) = server(&layout, "ready-only");
    let (event, ()) = tokio::join!(
        async {
            handle.try_inspect_worktree_teardown(3, &project).unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut host, health())
    );
    let WorkspaceEvent::WorktreeInspected {
        generation: 3,
        result,
        ..
    } = event
    else {
        panic!("ready-only host was not inspected: {event:?}");
    };
    let row = (*result).unwrap().expect("ready-only host should be live");
    assert_eq!(row.project_root, project);
    assert!(row.running);
    assert_eq!(row.unsaved_buffers, Some(0));
    let reviewed_live = row.selection();

    let (event, ()) = tokio::join!(
        async {
            handle
                .try_prepare_worktree_teardown(4, &project, None)
                .unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut host, health())
    );
    let WorkspaceEvent::WorktreePrepared { result, .. } = event else {
        panic!("ready-only host preparation returned the wrong event");
    };
    assert!(matches!(*result, Err(error) if error.contains("changed after confirmation")));
    host.shutdown().await.unwrap();
    let (_, mut replacement) = server(&layout, "replacement");
    let (event, ()) = tokio::join!(
        async {
            handle
                .try_prepare_worktree_teardown(5, &project, Some(reviewed_live))
                .unwrap();
            events.recv().await.unwrap()
        },
        answer(&mut replacement, health())
    );
    let WorkspaceEvent::WorktreePrepared { result, .. } = event else {
        panic!("replacement host preparation returned the wrong event");
    };
    assert!(matches!(*result, Err(error) if error.contains("changed after confirmation")));
    owner.shutdown().await.unwrap();
    replacement.shutdown().await.unwrap();
}

#[tokio::test]
async fn selected_stopped_row_refuses_a_new_live_host_and_cancels_provisional_startup() {
    let Some(_controlled_job) = run_in_controlled_parent_attach_job(
        "selected_stopped_row_refuses_a_new_live_host_and_cancels_provisional_startup",
    ) else {
        return;
    };
    let root = TestRuntimeRoot::new("native-service-selected-stopped").unwrap();
    let source = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    let destination_layout = source
        .discovery_scope()
        .initialize_layout(&destination, Path::new(".runyte"))
        .unwrap();
    crate::workspace::windows_catalog::ensure_recorded(&destination_layout)
        .unwrap()
        .unwrap();
    let startup = ParentAttachStartup::capture(&std::env::current_exe().unwrap(), None, 0, None)
        .unwrap()
        .with_test_harness_helper(SILENT_PARENT_ATTACH_FIXTURE);
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        source.discovery_scope().clone(),
        Some(source.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();
    handle.try_refresh(1, false).unwrap();
    let WorkspaceEvent::Refreshed {
        result: Ok(rows), ..
    } = events.recv().await.unwrap()
    else {
        panic!("stopped selection did not appear in the catalog");
    };
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].running);
    let selection = rows[0].selection();
    assert!(
        handle
            .prepare_selected_session(selection.clone(), true)
            .await
            .is_err()
    );

    let (_, mut replacement) = server(&destination_layout, "replacement");
    let (result, ()) = tokio::join!(
        handle.prepare_selected_session(selection.clone(), false),
        answer(&mut replacement, health())
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("choose it again"), "{error}");
    replacement.shutdown().await.unwrap();

    let prepared = handle
        .prepare_selected_session(selection, false)
        .await
        .unwrap();
    let process = Arc::clone(prepared.peer());
    let decision = prepared
        .prepare_acceptance(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    drop(decision);
    tokio::time::timeout(Duration::from_secs(12), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(!process.is_alive().unwrap());
    assert!(
        destination_layout
            .publication_location()
            .unwrap()
            .observe_ready()
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn selected_stopped_startup_refuses_a_competing_existing_winner() {
    let root = TestRuntimeRoot::new("native-selected-stopped-race").unwrap();
    let source = layout(&root, "source", "cache");
    let destination = layout(&root, "destination", "cache");
    let (_, mut replacement) = server(&destination, "winner");
    let startup =
        ParentAttachStartup::capture(&root.join("missing-startup.exe"), None, 0, None).unwrap();
    let (_requests, request_rx) = mpsc::channel(1);
    let (_preview_sender, preview_rx) = watch::channel(None);
    let (events, _event_rx) = mpsc::channel(1);
    let (_stop_sender, stop_rx) = watch::channel(false);
    let mut worker = Worker {
        scope: source.discovery_scope().clone(),
        current: Some(source.read_location()),
        current_layout: None,
        configured_state: PathBuf::from(".runyte"),
        parent_attach: Some(startup),
        snapshot: None,
        pending_worktree_teardown: None,
        include_hidden: false,
        requests: request_rx,
        previews: preview_rx,
        events,
        stop: stop_rx,
    };
    let (mut reply, _receiver) = oneshot::channel();
    let result = tokio::time::timeout(Duration::from_secs(12), async {
        let preparation =
            worker.prepare_startup_for_project(destination.project_root(), &mut reply, true);
        tokio::pin!(preparation);
        loop {
            tokio::select! {
                biased;
                result = &mut preparation => break result,
                event = replacement.recv() => match event.unwrap() {
                    ServerEvent::Connected { responses, .. } => {
                        responses.send(HostResponse::Welcome {
                            protocol: VERSION,
                            pid: std::process::id(),
                            features: vec![
                                FeatureGroup::Control,
                                FeatureGroup::Buffers,
                                FeatureGroup::Wait,
                            ],
                            host_version: env!("CARGO_PKG_VERSION").to_owned(),
                        }).await.unwrap();
                    }
                    ServerEvent::Disconnected { .. } => {}
                    other => panic!("unexpected winner readiness event: {other:?}"),
                },
            }
        }
    })
    .await
    .unwrap();
    let error = result
        .err()
        .expect("competing winner was accepted")
        .to_string();
    assert!(error.contains("choose it again"), "{error}");
    assert!(
        PinnedProcess::open_peer(replacement.metadata().process.pid)
            .unwrap()
            .is_alive()
            .unwrap()
    );
    replacement.shutdown().await.unwrap();
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
            handle.try_inventory(31, old_selection.clone()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        },
        answer(&mut replacement, health())
    );
    assert!(matches!(
        event,
        WorkspaceEvent::Inventory {
            generation: 31,
            selection,
            result: Err(error),
            ..
        } if selection == old_selection && error.contains("choose it again")
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
async fn parent_attach_selector_retains_the_fresh_exact_live_publication() {
    let root = TestRuntimeRoot::new("native-service-parent-live").unwrap();
    let layout = layout(&root, "project", "cache");
    let (_, mut host) = server(&layout, "destination");
    let startup =
        ParentAttachStartup::capture(&root.join("missing-startup.exe"), None, 0, None).unwrap();
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();

    let (prepared, ()) = tokio::join!(
        handle.prepare_parent_attach(Path::new("destination"), root.path()),
        answer(&mut host, health())
    );
    let prepared = prepared.unwrap();
    assert_eq!(prepared.metadata().address, host.metadata().address);
    assert_eq!(prepared.peer().identity(), prepared.metadata().process);
    assert!(prepared.peer().is_alive().unwrap());
    drop(prepared);
    assert!(
        PinnedProcess::open_peer(host.metadata().process.pid)
            .unwrap()
            .is_alive()
            .unwrap(),
        "abandoning a prepared existing winner must not retire it"
    );

    host.shutdown().await.unwrap();
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn parent_attach_initializes_an_unseen_exact_directory_before_startup() {
    let root = TestRuntimeRoot::new("native-service-parent-initialize").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    let missing = root.join("missing-startup.exe");
    let startup = ParentAttachStartup::capture(&missing, None, 2, None).unwrap();
    assert!(startup.executable.is_absolute());
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();

    let error = handle
        .prepare_parent_attach(Path::new("destination"), root.path())
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<windows_startup::UnavailableStartupExecutable>()
            .is_some(),
        "{error:#}"
    );
    assert!(destination.join(".runyte").is_dir());
    assert!(!missing.exists());

    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn abandoned_queued_parent_attach_is_skipped_without_initialization() {
    let root = TestRuntimeRoot::new("native-service-parent-cancelled").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    let startup =
        ParentAttachStartup::capture(&root.join("missing-startup.exe"), None, 0, None).unwrap();
    let (handle, mut owner, mut events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();
    let (release, hold) = oneshot::channel();
    let (entered, entered_rx) = oneshot::channel();
    handle
        .submit(Request::Hold {
            entered,
            release: hold,
        })
        .unwrap();
    entered_rx.await.unwrap();
    let (reply, abandoned) = oneshot::channel();
    handle
        .submit(Request::PrepareParentAttach {
            selector: destination.clone(),
            working_directory: root.path().to_owned(),
            reply,
        })
        .unwrap();
    drop(abandoned);
    handle
        .submit(Request::Emit {
            generation: 41,
            entered: None,
        })
        .unwrap();
    release.send(()).unwrap();

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::Refreshed { generation: 41, .. }
    ));
    assert!(!destination.join(".runyte").exists());

    owner.shutdown().await.unwrap();
}

const SILENT_PARENT_ATTACH_FIXTURE: &str =
    "workspace::windows_service::tests::silent_parent_attach_fixture";
const PARENT_ATTACH_DESCENDANT_FIXTURE: &str =
    "workspace::windows_service::tests::parent_attach_descendant_fixture";
const PARENT_ATTACH_CONTROLLED_CASE: &str = "RUNYTE_TEST_PARENT_ATTACH_CONTROLLED_CASE";
const PARENT_ATTACH_CONTROLLED_ROOT: &str = "RUNYTE_TEST_PARENT_ATTACH_CONTROLLED_ROOT";
const PARENT_ATTACH_CONTROLLED_DIAGNOSTIC_BYTES: usize = 4096;

struct ControlledParentAttachProcess {
    root: Option<TestRuntimeRoot>,
    child: Option<crate::windows_process::Child>,
}

impl ControlledParentAttachProcess {
    fn new(root: TestRuntimeRoot, child: crate::windows_process::Child) -> Self {
        Self {
            root: Some(root),
            child: Some(child),
        }
    }

    fn root(&self) -> &TestRuntimeRoot {
        self.root.as_ref().expect("controlled fixture root")
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.as_mut().expect("controlled child").try_wait()
    }

    fn settle(&mut self) -> std::io::Result<()> {
        let result = self
            .child
            .as_mut()
            .expect("controlled child")
            .terminate_and_wait_tree()
            .map(drop);
        if result.is_ok() {
            self.child.take();
        }
        result
    }
}

impl Drop for ControlledParentAttachProcess {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let result = child.terminate_and_wait_tree();
        if result.is_ok() {
            self.child.take();
        } else if let Some(root) = self.root.take() {
            // Keep diagnostic and publication storage when process-tree
            // settlement could not be proved. Removing it could race a live
            // descendant and would erase the evidence for the failure.
            std::mem::forget(root);
        }
        if !std::thread::panicking() {
            assert!(
                result.is_ok(),
                "controlled parent attach process tree did not settle: {result:?}"
            );
        }
    }
}

/// Cargo and other supervisors may put libtest in a job that forbids the
/// explicit breakaway required by detached startup. Run each spawning case in
/// an owned fixture job, then give only that child an immediate job that
/// permits the startup layer's explicit breakaway. The owned outer job remains
/// the cleanup boundary for every process in the case.
fn run_in_controlled_parent_attach_job(case: &str) -> Option<OwnedHandle> {
    if std::env::var(PARENT_ATTACH_CONTROLLED_CASE).as_deref() == Ok(case) {
        let root = PathBuf::from(std::env::var_os(PARENT_ATTACH_CONTROLLED_ROOT).unwrap());
        std::panic::set_hook(Box::new(move |info| {
            let detail = bounded(info.to_string(), PARENT_ATTACH_CONTROLLED_DIAGNOSTIC_BYTES);
            let _ = fs::write(root.join("controlled-failure.txt"), detail);
        }));
        return Some(enter_parent_attach_breakaway_job());
    }

    let root = TestRuntimeRoot::new("controlled-parent-attach-service").unwrap();
    let exact = format!("workspace::windows_service::tests::{case}");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args(["--exact", &exact, "--nocapture", "--test-threads=1"])
        .current_dir(root.path())
        .env(PARENT_ATTACH_CONTROLLED_CASE, case)
        .env(PARENT_ATTACH_CONTROLLED_ROOT, root.path())
        .env("XDG_CONFIG_HOME", root.join("config"));
    let input: OwnedHandle = fs::OpenOptions::new()
        .read(true)
        .open("NUL")
        .unwrap()
        .into();
    let output: OwnedHandle = fs::OpenOptions::new()
        .write(true)
        .open("NUL")
        .unwrap()
        .into();
    let error: OwnedHandle = fs::OpenOptions::new()
        .write(true)
        .open("NUL")
        .unwrap()
        .into();
    let child = crate::windows_process::spawn_with_stdio(&command, [input, output, error]).unwrap();
    // The guard owns both the job and its storage. It removes storage only
    // after whole-tree settlement has been proved.
    let mut child = ControlledParentAttachProcess::new(root, child);
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    child.settle().unwrap();
    let mut diagnostic = Vec::new();
    if let Ok(file) = fs::File::open(child.root().join("controlled-failure.txt")) {
        file.take(PARENT_ATTACH_CONTROLLED_DIAGNOSTIC_BYTES as u64)
            .read_to_end(&mut diagnostic)
            .unwrap();
    }
    assert!(
        status.is_some_and(|status| status.success()),
        "controlled parent attach service case {case} failed ({status:?}): {}",
        String::from_utf8_lossy(&diagnostic),
    );
    None
}

fn enter_parent_attach_breakaway_job() -> OwnedHandle {
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    assert!(!raw.is_null());
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    assert_ne!(
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        },
        0
    );
    assert_ne!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) },
        0
    );
    job
}

struct ReleasedParentFixtureGuard {
    project: PathBuf,
    process: Arc<PinnedProcess>,
    terminate: std::os::windows::io::OwnedHandle,
}

impl ReleasedParentFixtureGuard {
    fn new(project: PathBuf, process: Arc<PinnedProcess>) -> Self {
        use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };

        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                0,
                process.identity().pid,
            )
        };
        assert!(
            !handle.is_null(),
            "open released fixture termination handle"
        );
        let terminate = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) };
        assert_ne!(
            unsafe {
                windows_sys::Win32::Foundation::CompareObjectHandles(
                    terminate.as_raw_handle(),
                    process.as_handle().as_raw_handle(),
                )
            },
            0,
            "termination guard must retain the exact released fixture process"
        );
        Self {
            project,
            process,
            terminate,
        }
    }
}

impl Drop for ReleasedParentFixtureGuard {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{TerminateProcess, WaitForSingleObject},
        };

        let _ = fs::write(self.project.join("allow-parent-commit-ack"), "continue");
        let state = unsafe { WaitForSingleObject(self.terminate.as_raw_handle(), 0) };
        assert!(matches!(state, WAIT_OBJECT_0 | WAIT_TIMEOUT));
        if state == WAIT_TIMEOUT {
            assert_ne!(
                unsafe { TerminateProcess(self.terminate.as_raw_handle(), 1) },
                0
            );
        }
        let settled = unsafe { WaitForSingleObject(self.terminate.as_raw_handle(), 5000) };
        assert_eq!(settled, WAIT_OBJECT_0, "released fixture did not terminate");
        assert!(!self.process.is_alive().unwrap_or(false));
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) {
    let pending = path.with_extension("pending");
    fs::write(&pending, bytes).unwrap();
    fs::rename(pending, path).unwrap();
}

#[test]
#[ignore = "surviving descendant of a provisional ParentAttach startup"]
fn parent_attach_descendant_fixture() {
    let project = PathBuf::from(std::env::var_os("RUNYTE_TEST_PARENT_ATTACH_PROJECT").unwrap());
    write_atomic(
        &project.join("silent-parent-descendant-pid"),
        std::process::id().to_string().as_bytes(),
    );
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
#[ignore = "compiled child fixture for service-owned native startup"]
fn silent_parent_attach_fixture() {
    let project = PathBuf::from(std::env::var_os("RUNYTE_TEST_PARENT_ATTACH_PROJECT").unwrap());
    let state = PathBuf::from(std::env::var_os("RUNYTE_TEST_PARENT_ATTACH_STATE").unwrap());
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots: Vec::new(),
        roots: CapturedRoots::capture(),
    })
    .unwrap();
    let layout = ResolvedLayout::from_scope(scope, &project, state).unwrap();
    write_atomic(
        &project.join("silent-parent-pid"),
        std::process::id().to_string().as_bytes(),
    );
    let _descendant = project.join("spawn-parent-descendant").exists().then(|| {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                PARENT_ATTACH_DESCENDANT_FIXTURE,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RUNYTE_TEST_PARENT_ATTACH_PROJECT", &project)
            .env("XDG_CONFIG_HOME", project.join(".test-config"))
            .spawn()
            .unwrap()
    });
    while project.join("hold-parent-ready").exists() && !project.join("allow-parent-ready").exists()
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let endpoint = layout.publication_location().unwrap();
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        let mut responses = Vec::new();
        loop {
            let event = server.recv().await.unwrap();
            match event {
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
                    responses.push(sender);
                }
                ServerEvent::Request {
                    request: ClientRequest::Shutdown,
                    ..
                } => return,
                ServerEvent::Disconnected { .. } | ServerEvent::TransportFailure { .. } => {}
                _ => {}
            }
        }
    });
}

#[tokio::test]
async fn cancellation_immediately_before_readiness_settles_the_provisional_job() {
    let Some(_controlled_job) = run_in_controlled_parent_attach_job(
        "cancellation_immediately_before_readiness_settles_the_provisional_job",
    ) else {
        return;
    };
    let root = TestRuntimeRoot::new("native-service-parent-provisional").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    fs::write(destination.join("hold-parent-ready"), "hold").unwrap();
    let startup = ParentAttachStartup::capture(&std::env::current_exe().unwrap(), None, 0, None)
        .unwrap()
        .with_test_harness_helper(SILENT_PARENT_ATTACH_FIXTURE);
    let scope = layout.discovery_scope().clone();
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        scope.clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();
    let preparing = tokio::spawn({
        let destination = destination.clone();
        let working_directory = root.path().to_owned();
        async move {
            handle
                .prepare_parent_attach(&destination, &working_directory)
                .await
        }
    });
    let pid_path = destination.join("silent-parent-pid");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !pid_path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let pid = fs::read_to_string(&pid_path).unwrap().parse().unwrap();
    let process = PinnedProcess::open_peer(pid).unwrap();
    preparing.abort();
    let _ = preparing.await;
    fs::write(destination.join("allow-parent-ready"), "continue").unwrap();

    tokio::time::timeout(Duration::from_secs(12), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(!process.is_alive().unwrap());
    let destination_layout = scope
        .initialize_layout(&destination, Path::new(".runyte"))
        .unwrap();
    assert!(
        destination_layout
            .publication_location()
            .unwrap()
            .observe_ready()
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn abandoned_parent_attach_result_settles_the_provisional_job() {
    let Some(_controlled_job) = run_in_controlled_parent_attach_job(
        "abandoned_parent_attach_result_settles_the_provisional_job",
    ) else {
        return;
    };
    let root = TestRuntimeRoot::new("native-service-parent-result-abandoned").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    let startup = ParentAttachStartup::capture(&std::env::current_exe().unwrap(), None, 0, None)
        .unwrap()
        .with_test_harness_helper(SILENT_PARENT_ATTACH_FIXTURE);
    let scope = layout.discovery_scope().clone();
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        scope.clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();

    let prepared = handle
        .prepare_parent_attach(&destination, root.path())
        .await
        .unwrap();
    let process = Arc::clone(prepared.peer());
    drop(prepared);

    tokio::time::timeout(Duration::from_secs(12), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(!process.is_alive().unwrap());
    let destination_layout = scope
        .initialize_layout(&destination, Path::new(".runyte"))
        .unwrap();
    assert!(
        destination_layout
            .publication_location()
            .unwrap()
            .observe_ready()
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cancelled_prepared_acceptance_settles_before_release() {
    let Some(_controlled_job) =
        run_in_controlled_parent_attach_job("cancelled_prepared_acceptance_settles_before_release")
    else {
        return;
    };
    let root = TestRuntimeRoot::new("native-service-parent-accept-cancel").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    let startup = ParentAttachStartup::capture(&std::env::current_exe().unwrap(), None, 0, None)
        .unwrap()
        .with_test_harness_helper(SILENT_PARENT_ATTACH_FIXTURE);
    let scope = layout.discovery_scope().clone();
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        scope.clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();

    let prepared = handle
        .prepare_parent_attach(&destination, root.path())
        .await
        .unwrap();
    let process = Arc::clone(prepared.peer());
    let decision = prepared
        .prepare_acceptance(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    drop(decision);

    tokio::time::timeout(Duration::from_secs(12), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(!process.is_alive().unwrap());
    let destination_layout = scope
        .initialize_layout(&destination, Path::new(".runyte"))
        .unwrap();
    assert!(
        destination_layout
            .publication_location()
            .unwrap()
            .observe_ready()
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn dropped_ack_after_atomic_commit_keeps_released_destination() {
    let Some(_controlled_job) = run_in_controlled_parent_attach_job(
        "dropped_ack_after_atomic_commit_keeps_released_destination",
    ) else {
        return;
    };
    let root = TestRuntimeRoot::new("native-service-parent-committed-ack-loss").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    fs::write(destination.join("hold-parent-commit-ack"), "hold").unwrap();
    let startup = ParentAttachStartup::capture(&std::env::current_exe().unwrap(), None, 0, None)
        .unwrap()
        .with_test_harness_helper(SILENT_PARENT_ATTACH_FIXTURE);
    let scope = layout.discovery_scope().clone();
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        scope.clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();

    let prepared = handle
        .prepare_parent_attach(&destination, root.path())
        .await
        .unwrap();
    let process = Arc::clone(prepared.peer());
    let released = ReleasedParentFixtureGuard::new(destination.clone(), Arc::clone(&process));
    let committed = prepared
        .prepare_acceptance(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap()
        .commit()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !destination.join("parent-released").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(committed);
    fs::write(destination.join("allow-parent-commit-ack"), "continue").unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        while !process.is_alive().unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    owner.shutdown().await.unwrap();
    assert!(process.is_alive().unwrap());
    let destination_layout = scope
        .initialize_layout(&destination, Path::new(".runyte"))
        .unwrap();
    assert!(
        destination_layout
            .publication_location()
            .unwrap()
            .observe_ready()
            .unwrap()
            .is_some()
    );
    drop(released);
    assert!(!process.is_alive().unwrap());
}

#[tokio::test]
async fn service_shutdown_settles_an_unaccepted_parent_attach_job() {
    let Some(_controlled_job) = run_in_controlled_parent_attach_job(
        "service_shutdown_settles_an_unaccepted_parent_attach_job",
    ) else {
        return;
    };
    let root = TestRuntimeRoot::new("native-service-parent-shutdown").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    let startup = ParentAttachStartup::capture(&std::env::current_exe().unwrap(), None, 0, None)
        .unwrap()
        .with_test_harness_helper(SILENT_PARENT_ATTACH_FIXTURE);
    let scope = layout.discovery_scope().clone();
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        scope.clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();

    let prepared = handle
        .prepare_parent_attach(&destination, root.path())
        .await
        .unwrap();
    let process = Arc::clone(prepared.peer());
    tokio::time::timeout(Duration::from_secs(12), owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(!process.is_alive().unwrap());
    assert!(
        prepared
            .accept(std::time::Instant::now() + Duration::from_secs(1))
            .await
            .is_err()
    );
    let destination_layout = scope
        .initialize_layout(&destination, Path::new(".runyte"))
        .unwrap();
    assert!(
        destination_layout
            .publication_location()
            .unwrap()
            .observe_ready()
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn leader_exit_before_acceptance_reaps_descendant_and_publication() {
    let Some(_controlled_job) = run_in_controlled_parent_attach_job(
        "leader_exit_before_acceptance_reaps_descendant_and_publication",
    ) else {
        return;
    };
    let root = TestRuntimeRoot::new("native-service-parent-release-failure").unwrap();
    let layout = layout(&root, "source", "cache");
    let destination = root.create_private_dir("destination").unwrap();
    fs::write(destination.join("spawn-parent-descendant"), "spawn").unwrap();
    let startup = ParentAttachStartup::capture(&std::env::current_exe().unwrap(), None, 0, None)
        .unwrap()
        .with_test_harness_helper(SILENT_PARENT_ATTACH_FIXTURE);
    let scope = layout.discovery_scope().clone();
    let (handle, mut owner, _events) = WorkspaceServiceOwner::spawn_with_parent_attach(
        scope.clone(),
        Some(layout.read_location()),
        PathBuf::from(".runyte"),
        startup,
    )
    .unwrap();

    let prepared = handle
        .prepare_parent_attach(&destination, root.path())
        .await
        .unwrap();
    let leader = Arc::clone(prepared.peer());
    let descendant_pid_path = destination.join("silent-parent-descendant-pid");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !descendant_pid_path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let descendant_pid = fs::read_to_string(descendant_pid_path)
        .unwrap()
        .parse()
        .unwrap();
    let descendant = Arc::new(PinnedProcess::open_peer(descendant_pid).unwrap());
    let descendant_exit = crate::workspace::NativeHostExit::new(Arc::clone(&descendant)).unwrap();
    let leader_exit = crate::workspace::NativeHostExit::new(Arc::clone(&leader)).unwrap();
    let mut control = connect_control(prepared.metadata()).await.unwrap();
    control.send(&ClientRequest::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), leader_exit.wait())
        .await
        .expect("parent attach fixture ignored its authenticated shutdown request");
    drop(control);
    assert!(descendant.is_alive().unwrap());

    let error = prepared
        .accept(std::time::Instant::now() + Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("release"), "{error:#}");
    // Failed release settlement has already proved the provisional job empty.
    // Job accounting can reach zero just before a retained descendant process
    // handle becomes signaled, so wait on that exact object before inspecting
    // its exit state.
    tokio::time::timeout(Duration::from_secs(5), descendant_exit.wait())
        .await
        .expect("provisional descendant handle was not signaled after job cleanup");
    assert!(!descendant.is_alive().unwrap());
    let destination_layout = scope
        .initialize_layout(&destination, Path::new(".runyte"))
        .unwrap();
    assert!(
        destination_layout
            .publication_location()
            .unwrap()
            .observe_ready()
            .unwrap()
            .is_none()
    );

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
