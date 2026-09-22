// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    private_storage::Directory,
    protocol::{FeatureGroup, VERSION},
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{CandidateOrigin, EndpointLocation, RegistryRecord, ScanLimit},
        windows_location::{CapturedRoots, LocationInputs},
        windows_transport::{LocalServer, ServerEvent},
    },
};
use std::{ffi::OsStr, fs};

pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
pub(crate) fn layout(root: &TestRuntimeRoot, project: &str, namespace: &str) -> ResolvedLayout {
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
pub(crate) fn server(layout: &ResolvedLayout, name: &str) -> (EndpointLocation, LocalServer) {
    let endpoint = layout.publication_location().unwrap();
    let server = LocalServer::bind(endpoint.prepare(Some(name.into())).unwrap()).unwrap();
    (endpoint, server)
}
pub(crate) fn health() -> HostResponse {
    HostResponse::Health {
        protocol: VERSION,
        pid: std::process::id(),
        interactive_attached: false,
        unsaved_buffers: 2,
        open_buffers: 3,
        pending_wait_requests: 1,
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
pub(crate) async fn answer(server: &mut LocalServer, health: HostResponse) {
    let mut responses = None;
    timeout_at(Instant::now() + Duration::from_secs(5), async {
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
                            host_version: env!("CARGO_PKG_VERSION").into(),
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
                other => panic!("unexpected fixture request: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
}

#[test]
fn replica_sources_form_one_authenticated_entry_with_health_and_no_history_writes() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-replicas").unwrap();
        let layout = layout(&root, "project", "cache");
        let (_endpoint, mut host) = server(&layout, "native");
        let (result, ()) = tokio::join!(snapshot(&layout, &[], true), answer(&mut host, health()));
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 1);
        let entry = &result.entries()[0];
        assert_eq!(entry.observations().len(), 3);
        assert_eq!(entry.peer().identity(), host.metadata().process);
        assert_eq!(entry.row().unsaved_buffers, Some(2));
        assert_eq!(entry.row().pending_wait_requests, Some(1));
        assert!(!entry.row().missing_directory);
        assert!(result.absent_projects().is_empty());
        assert!(!layout.name_store_root().exists());
        assert!(!root.join("cache/runyte/workspaces.json").exists());
        assert_eq!(
            result
                .select(Path::new("native"), None)
                .unwrap()
                .unwrap()
                .metadata(),
            host.metadata()
        );
        host.shutdown().await.unwrap();
    });
}

#[test]
fn isolated_same_project_publications_remain_ambiguous_by_project_or_id() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-distinct").unwrap();
        let left = layout(&root, "project", "cache-a");
        let mut right = layout(&root, "project", "cache-b");
        // Separate ready placement is an explicit configuration choice.
        right = ResolvedLayout::resolve(LocationInputs {
            project_root: right.project_root().into(),
            state_root: root.join("state-b"),
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                cache_home: Some(root.join("cache-b")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        let (_, mut a) = server(&left, "left");
        let (_, mut b) = server(&right, "right");
        let (result, (), ()) = tokio::join!(
            snapshot(&left, &[], true),
            answer(&mut a, health()),
            answer(&mut b, health())
        );
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 2);
        assert!(result.select(left.project_root(), None).is_err());
        assert!(result.select(Path::new(&a.metadata().id), None).is_err());
        assert!(
            result
                .select(Path::new(&a.metadata().id[..6]), None)
                .is_err()
        );
        assert_eq!(
            result
                .select(Path::new("right"), None)
                .unwrap()
                .unwrap()
                .metadata(),
            b.metadata()
        );
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
    });
}

#[test]
fn known_ready_host_is_found_when_its_registry_row_is_missing() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-ready").unwrap();
        let current = layout(&root, "current", "cache");
        let known = layout(&root, "known", "cache");
        let (_, mut host) = server(&known, "ready-only");
        fs::remove_file(known.namespace_roots()[0].join(format!("{}.json", host.metadata().id)))
            .unwrap();
        let (result, ()) = tokio::join!(
            snapshot(&current, std::slice::from_ref(&known), false),
            answer(&mut host, health())
        );
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 1);
        assert_eq!(
            result.entries()[0].observations()[0].origin(),
            CandidateOrigin::ConfiguredReady
        );
        assert_eq!(
            result.absent_projects(),
            &[current.project_root().to_owned()]
        );
        host.shutdown().await.unwrap();
    });
}

#[test]
fn hidden_inventory_host_does_not_invent_namespace_or_ready_locations() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-hidden").unwrap();
        let visible = layout(&root, "visible", "cache-a");
        let project = root.join("hidden");
        fs::create_dir(&project).unwrap();
        let hidden = ResolvedLayout::resolve(LocationInputs {
            project_root: project,
            state_root: root.join("hidden-state"),
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                cache_home: Some(root.join("cache-b")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        let (_, mut host) = server(&hidden, "hidden-session");
        // The publication stays outside the project, so removal does not rely
        // on Windows permitting a directory rename with open child handles.
        fs::remove_dir(hidden.project_root()).unwrap();
        let (result, ()) = tokio::join!(snapshot(&visible, &[], true), answer(&mut host, health()));
        let result = result.unwrap();
        let entry = &result.entries()[0];
        assert!(entry.row().missing_directory);
        assert_eq!(entry.observations().len(), 1);
        assert_eq!(
            entry.observations()[0].origin(),
            CandidateOrigin::OwnerInventory
        );
        assert!(!visible.endpoint_directory().exists());
        assert_eq!(
            result
                .select(hidden.project_root(), None)
                .unwrap()
                .unwrap()
                .metadata(),
            host.metadata()
        );
        assert!(
            result
                .select(Path::new("hidden"), Some(root.path()))
                .unwrap()
                .is_some()
        );
        assert!(result.select(&root.join("hidden"), None).unwrap().is_some());
        // Catalog observation does not reconstruct or remove the hidden
        // ready file; the actual host retains its own independent cleanup owner.
        assert!(root.join("hidden-state/host/endpoint.json").exists());
        host.shutdown().await.unwrap();
    });
}

#[test]
fn conflicting_replicas_refuse_a_complete_snapshot_before_host_queries() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-conflict").unwrap();
        let layout = layout(&root, "project", "cache");
        let (_, mut host) = server(&layout, "old");
        let mut metadata = host.metadata().clone();
        metadata.name = Some("different".into());
        Directory::open_existing(layout.endpoint_directory(), true)
            .unwrap()
            .atomic_write(
                OsStr::new("endpoint.json"),
                &serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        let error = snapshot(&layout, &[], false).await.unwrap_err();
        assert!(error.to_string().contains("copies disagree"));
        assert_eq!(
            fs::read(layout.endpoint_directory().join("endpoint.json")).unwrap(),
            serde_json::to_vec(&metadata).unwrap()
        );
        host.shutdown().await.unwrap();
    });
}

#[test]
fn missing_pipe_and_wrong_health_identity_are_uncertainty_not_stopped() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-uncertain").unwrap();
        let layout = layout(&root, "project", "cache");
        let endpoint = layout.publication_location().unwrap();
        let publication = endpoint.prepare(None).unwrap().publish().unwrap();
        assert!(snapshot(&layout, &[], false).await.is_err());
        assert!(endpoint.read_ready().unwrap().is_some());
        drop(publication);
        let (_, mut host) = server(&layout, "wrong-health");
        let mut wrong = health();
        if let HostResponse::Health { pid, .. } = &mut wrong {
            *pid = pid.wrapping_add(1);
        }
        let (result, ()) = tokio::join!(snapshot(&layout, &[], false), answer(&mut host, wrong));
        assert!(result.unwrap_err().to_string().contains("health identity"));
        host.shutdown().await.unwrap();
    });
}

#[test]
fn scan_limits_and_corrupt_known_ready_are_errors_without_mutation() {
    runtime().block_on(async {
        for limit in [
            ScanLimit::Entries,
            ScanLimit::Rows,
            ScanLimit::MetadataBytes,
        ] {
            let mut observations = Vec::new();
            assert!(
                add_scan(
                    &mut observations,
                    Scan {
                        limit: Some(limit),
                        ..Scan::default()
                    }
                )
                .is_err()
            );
            assert!(observations.is_empty());
        }
        let root = TestRuntimeRoot::new("catalog-malformed").unwrap();
        let layout = layout(&root, "project", "cache");
        let directory = Directory::open(layout.endpoint_directory(), true).unwrap();
        directory
            .atomic_write(OsStr::new("endpoint.json"), b"malformed")
            .unwrap();
        assert!(snapshot(&layout, &[], false).await.is_err());
        assert_eq!(
            fs::read(layout.endpoint_directory().join("endpoint.json")).unwrap(),
            b"malformed"
        );
        assert!(!root.join("inventory").exists());
    });
}

#[test]
fn health_retention_and_expired_probe_budgets_fail_without_a_partial_result() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-budgets").unwrap();
        let layout = layout(&root, "project", "cache");
        let endpoint = layout.publication_location().unwrap();
        let publication = endpoint.prepare(None).unwrap().publish().unwrap();
        let view = layout.discovery_view(false).unwrap();
        let candidates = view.scan_namespaces().candidates;
        assert!(
            build_snapshot(candidates, vec![], Instant::now())
                .await
                .unwrap_err()
                .to_string()
                .contains("budget exhausted")
        );
        let mut value = row(publication.metadata()).unwrap();
        let mut used = MAX_HEALTH_BYTES;
        let mut response = health();
        if let HostResponse::Health { activities, .. } = &mut response {
            *activities = vec![crate::protocol::ActivityLeaseHealth {
                owner: "fixture".into(),
                title: "active".into(),
                state: crate::protocol::ActivityLeaseState::Active,
            }]
            .into_boxed_slice();
        }
        assert!(apply_health(&mut value, response, std::process::id(), &mut used).is_err());
        assert!(value.unsaved_buffers.is_none());
    });
}

#[test]
fn conclusive_stale_records_are_retained_without_cleanup_or_live_claims() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-stale").unwrap();
        let layout = layout(&root, "project", "cache");
        let endpoint = layout.publication_location().unwrap();
        let publication = endpoint.prepare(None).unwrap().publish().unwrap();
        let mut metadata = publication.metadata().clone();
        metadata.process.creation_time ^= 1;
        let record = RegistryRecord {
            host: metadata.clone(),
            ready_record_bytes: crate::native_path::encode_path(&endpoint.ready_record()),
        };
        Directory::open_existing(&layout.namespace_roots()[0], true)
            .unwrap()
            .atomic_write(
                OsStr::new(&format!("{}.json", metadata.id)),
                &serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
        Directory::open_existing(layout.endpoint_directory(), true)
            .unwrap()
            .atomic_write(
                OsStr::new("endpoint.json"),
                &serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        let result = snapshot(&layout, &[], false).await.unwrap();
        assert!(result.entries().is_empty());
        assert_eq!(result.stale_observations().len(), 2);
        assert_eq!(
            result.absent_projects(),
            &[layout.project_root().to_owned()]
        );
        assert!(endpoint.read_ready().unwrap().is_some());
    });
}

#[test]
fn incompatible_live_rows_require_actual_pipe_proof_and_have_unknown_health() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("catalog-incompatible").unwrap();
        let layout = layout(&root, "project", "cache");
        let (endpoint, mut host) = server(&layout, "older");
        let mut metadata = host.metadata().clone();
        metadata.protocol += 1;
        Directory::open_existing(layout.endpoint_directory(), true)
            .unwrap()
            .atomic_write(
                OsStr::new("endpoint.json"),
                &serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        let candidate = endpoint.observe_ready().unwrap().unwrap();
        let result = build_snapshot(vec![candidate], vec![], Instant::now() + SNAPSHOT_BUDGET)
            .await
            .unwrap();
        let entry = &result.entries()[0];
        assert_eq!(entry.row().incompatible_protocol, Some(VERSION + 1));
        assert!(entry.row().unsaved_buffers.is_none());
        assert_eq!(entry.peer().identity(), host.metadata().process);
        host.shutdown().await.unwrap();
    });
}

#[test]
fn projectless_catalog_does_not_invent_current_ready_or_create_missing_roots() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("scope-empty-catalog").unwrap();
        let scope = DiscoveryScope::resolve(crate::workspace::windows_location::DiscoveryInputs {
            roots: CapturedRoots {
                cache_home: Some(root.join("cache")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
            reserved_user_roots: vec![root.join("config")],
        })
        .unwrap();
        let result = snapshot_in_scope(&scope, None, &[], false).await.unwrap();
        assert!(result.entries().is_empty());
        assert!(result.absent_projects().is_empty());
        assert!(!root.join("cache").exists());
        assert!(!root.join("inventory").exists());
        assert!(!root.join("config").exists());
        assert!(!root.join(".runyte").exists());
    });
}

#[test]
fn projectless_catalog_authenticates_registered_host_and_checks_optional_current_scope() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("scope-live-catalog").unwrap();
        let layout = layout(&root, "project", "cache");
        let (_, mut host) = server(&layout, "live");
        let (result, ()) = tokio::join!(
            snapshot_in_scope(layout.discovery_scope(), None, &[], false),
            answer(&mut host, health()),
        );
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 1);
        assert_eq!(result.entries()[0].observations().len(), 1);
        assert_eq!(
            result.entries()[0].observations()[0].origin(),
            CandidateOrigin::ConfiguredNamespace
        );
        assert!(result.absent_projects().is_empty());
        let foreign = super::tests::layout(&root, "other", "foreign-cache").read_location();
        let error = snapshot_in_scope(layout.discovery_scope(), Some(&foreign), &[], false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("different configured namespace"));
        let repeated = vec![layout.read_location(); MAX_KNOWN_PROJECTS];
        assert!(
            snapshot_in_scope(
                layout.discovery_scope(),
                Some(&layout.read_location()),
                &repeated,
                false
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("limit")
        );
        host.shutdown().await.unwrap();
    });
}

#[test]
fn projectless_inventory_only_scope_retains_hidden_peer_proof_without_ready_inference() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("scope-inventory-only").unwrap();
        let layout = layout(&root, "hidden", "host-cache");
        let (_, mut host) = server(&layout, "hidden");
        let scope = DiscoveryScope::resolve(crate::workspace::windows_location::DiscoveryInputs {
            roots: CapturedRoots {
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
            reserved_user_roots: vec![],
        })
        .unwrap();
        assert!(scope.namespace_roots().is_empty());
        assert!(scope.cache_root().unwrap().is_none());
        let (result, ()) = tokio::join!(
            snapshot_in_scope(&scope, None, &[], true),
            answer(&mut host, health()),
        );
        let result = result.unwrap();
        let entry = &result.entries()[0];
        assert_eq!(entry.observations().len(), 1);
        assert_eq!(
            entry.observations()[0].origin(),
            CandidateOrigin::OwnerInventory
        );
        assert_eq!(entry.peer().identity(), host.metadata().process);
        assert!(result.absent_projects().is_empty());
        assert!(!root.join(".runyte").exists());
        host.shutdown().await.unwrap();
    });
}
