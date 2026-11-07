// SPDX-License-Identifier: MPL-2.0

use super::super::tests::{answer, health, layout, runtime, server};
use super::*;
use crate::{
    private_storage::Directory,
    test_support::TestRuntimeRoot,
    workspace::{
        recent_history::{RECENT_LIMIT, encode_recents},
        windows_endpoint::Inspection,
        windows_location::{CapturedRoots, LocationInputs},
    },
};
use std::{ffi::OsStr, fs};

fn write_history(layout: &ResolvedLayout, entries: &[RecentEntry]) -> PathBuf {
    let cache = layout.cache_root().unwrap().unwrap();
    Directory::open(cache, true)
        .unwrap()
        .atomic_write(
            OsStr::new("workspaces.json"),
            &encode_recents(entries).unwrap(),
        )
        .unwrap();
    cache.join("workspaces.json")
}

fn remembered(path: &Path, name: Option<&str>, number: Option<u8>) -> RecentEntry {
    RecentEntry::new(path.to_owned(), name.map(str::to_owned), number, Some(42))
}

#[test]
fn missing_history_and_namespace_roots_are_not_created_by_refresh() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-no-create").unwrap();
        let layout = layout(&root, "project", "cache");
        let result = snapshot_with_history(&layout, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert!(result.entries().is_empty());
        assert!(result.remembered().is_empty());
        assert!(!root.join("cache").exists());
        assert!(!layout.state_root().exists());
        assert!(!root.join("inventory").exists());
    });
}

#[test]
fn current_exact_ready_is_live_without_registry_or_history_rows() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-current-ready").unwrap();
        let layout = layout(&root, "project", "cache");
        let (_, mut host) = server(&layout, "ready-only");
        for namespace in layout.namespace_roots() {
            fs::remove_file(namespace.join(format!("{}.json", host.metadata().id))).unwrap();
        }
        assert!(!root.join("cache/runyte/workspaces.json").exists());
        let absent = snapshot_with_history_in_scope(
            layout.discovery_scope(),
            None,
            Path::new(".runyte"),
            false,
        )
        .await
        .unwrap();
        assert!(absent.entries().is_empty());

        let current = layout.read_location();
        let (exact, ()) = tokio::join!(
            snapshot_with_history_in_scope(
                layout.discovery_scope(),
                Some(&current),
                Path::new(".runyte"),
                false,
            ),
            answer(&mut host, health()),
        );
        let exact = exact.unwrap();
        let Some(HistoryTarget::Live { publication, .. }) = exact
            .select(layout.project_root(), Some(layout.project_root()))
            .unwrap()
        else {
            panic!("exact ready host was not selected");
        };
        assert_eq!(publication.metadata(), host.metadata());
        assert_eq!(publication.observations().len(), 1);
        assert_eq!(
            publication.observations()[0].origin(),
            CandidateOrigin::ConfiguredReady
        );
        host.shutdown().await.unwrap();
    });
}

#[test]
fn selector_history_without_a_current_project_preserves_its_captured_cache() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("scope-history-cache").unwrap();
        let scope = DiscoveryScope::resolve(crate::workspace::windows_location::DiscoveryInputs {
            roots: CapturedRoots {
                cache_home: Some(root.join("cache")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
            reserved_user_roots: vec![root.join("config")],
        })
        .unwrap();
        let empty = snapshot_with_history_in_scope(&scope, None, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert!(empty.entries().is_empty());
        assert!(empty.live().absent_projects().is_empty());
        assert!(!root.join("cache").exists());
        assert!(!root.join("inventory").exists());
        let project = root
            .create_private_dir("remembered")
            .unwrap()
            .canonicalize()
            .unwrap();
        let cache = scope.cache_root().unwrap().unwrap();
        Directory::open(cache, true)
            .unwrap()
            .atomic_write(
                OsStr::new("workspaces.json"),
                &encode_recents(&[remembered(&project, Some("saved"), Some(3))]).unwrap(),
            )
            .unwrap();
        let history_path = cache.join("workspaces.json");
        let before = fs::read(&history_path).unwrap();
        let result = snapshot_with_history_in_scope(&scope, None, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert_eq!(result.entries().len(), 1);
        assert_eq!(result.history_path.as_deref(), Some(history_path.as_path()));
        assert_eq!(
            result.live().absent_projects(),
            std::slice::from_ref(&project)
        );
        assert!(matches!(
            result.select(Path::new("saved"), None).unwrap(),
            Some(HistoryTarget::Stopped { .. })
        ));
        assert_eq!(fs::read(&history_path).unwrap(), before);
        assert!(!history_path.with_extension("lock").exists());
        assert!(!project.join(".runyte").exists());
        assert!(!root.join(".runyte").exists());
        // A later independent scope cannot redirect this snapshot's explicit
        // history transaction into a newly selected cache.
        let other = DiscoveryScope::resolve(crate::workspace::windows_location::DiscoveryInputs {
            roots: CapturedRoots {
                cache_home: Some(root.join("other-cache")),
                ..CapturedRoots::default()
            },
            reserved_user_roots: vec![],
        })
        .unwrap();
        assert_eq!(result.persist().unwrap(), 1);
        assert_eq!(read_recents(Some(&history_path)).unwrap()[0].number, None);
        assert!(!other.cache_root().unwrap().unwrap().exists());
    });
}

#[test]
fn selector_history_observes_remembered_ready_without_current_or_registry_rows() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("scope-history-ready").unwrap();
        let project = root.create_private_dir("known").unwrap();
        let known = ResolvedLayout::resolve(LocationInputs {
            state_root: project.join(".runyte"),
            project_root: project,
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                runtime_root: Some(root.create_private_dir("runtime").unwrap()),
                cache_home: Some(root.join("cache")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        let (_, mut host) = server(&known, "ready-only");
        let path = write_history(
            &known,
            &[remembered(known.project_root(), Some("cached"), None)],
        );
        let before = fs::read(&path).unwrap();
        for namespace in known.namespace_roots() {
            fs::remove_file(namespace.join(format!("{}.json", host.metadata().id))).unwrap();
        }
        fs::remove_dir(known.project_root()).unwrap();
        let (result, ()) = tokio::join!(
            snapshot_with_history_in_scope(
                known.discovery_scope(),
                None,
                Path::new(".runyte"),
                false
            ),
            answer(&mut host, health()),
        );
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 1);
        assert!(result.live().absent_projects().is_empty());
        let Some(HistoryTarget::Live { row, publication }) = result.target(0) else {
            panic!("expected exact remembered ready host")
        };
        assert_eq!(row.name.as_deref(), Some("ready-only"));
        assert!(row.missing_directory);
        assert_eq!(publication.metadata(), host.metadata());
        assert_eq!(publication.observations().len(), 1);
        assert_eq!(
            publication.observations()[0].origin(),
            CandidateOrigin::ConfiguredReady
        );
        assert_eq!(fs::read(path).unwrap(), before);
        host.shutdown().await.unwrap();
    });
}

#[test]
fn complete_history_adds_only_existing_stopped_directories_and_preserves_bytes() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-stopped").unwrap();
        let current = layout(&root, "current", "cache");
        let stopped = root.create_private_dir("stopped").unwrap().canonicalize().unwrap();
        let missing = root.join("missing");
        let entries = [remembered(&stopped, Some("saved"), Some(3)), remembered(&missing, Some("away"), Some(4))];
        let path = write_history(&current, &entries);
        let before = fs::read(&path).unwrap();
        let result = snapshot_with_history(&current, Path::new(".runyte"), false).await.unwrap();
        assert_eq!(result.entries().len(), 1);
        assert_eq!(result.remembered(), &entries);
        let row = result.entries()[0].row();
        assert_eq!(row.name.as_deref(), Some("saved"));
        assert_eq!(row.number, None);
        assert_eq!(row.last_active_unix_seconds, Some(42));
        assert!(matches!(result.select(Path::new("saved"), None).unwrap(), Some(HistoryTarget::Stopped { row }) if row.project_root == stopped));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("lock").exists());
        assert!(!current.state_root().exists());
    });
}

#[test]
fn missing_project_ready_record_is_read_by_stored_identity_without_registry_or_publication_rights()
{
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-missing-live").unwrap();
        let runtime_root = root.create_private_dir("runtime").unwrap();
        let roots = CapturedRoots {
            runtime_root: Some(runtime_root.clone()),
            cache_home: Some(root.join("cache")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        };
        let make = |name: &str| {
            let project = root.create_private_dir(name).unwrap();
            ResolvedLayout::resolve(LocationInputs {
                state_root: project.join(".runyte"),
                project_root: project,
                reserved_user_roots: vec![root.join("config")],
                roots: roots.clone(),
            })
            .unwrap()
        };
        let current = make("current");
        let known = make("known");
        let (_, mut host) = server(&known, "live");
        let history = write_history(
            &current,
            &[remembered(known.project_root(), Some("old-name"), Some(2))],
        );
        let before = fs::read(&history).unwrap();
        for namespace in known.namespace_roots() {
            fs::remove_file(namespace.join(format!("{}.json", host.metadata().id))).unwrap();
        }
        fs::remove_dir(known.project_root()).unwrap();
        let (result, ()) = tokio::join!(
            snapshot_with_history(&current, Path::new(".runyte"), false),
            answer(&mut host, health())
        );
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 1);
        let Some(HistoryTarget::Live { row, publication }) =
            result.select(Path::new("live"), None).unwrap()
        else {
            panic!("missing-directory host was not retained")
        };
        assert!(row.missing_directory);
        assert_eq!(row.name.as_deref(), Some("live"));
        assert_eq!(row.number, Some(1));
        assert_eq!(publication.metadata(), host.metadata());
        assert_eq!(publication.observations().len(), 1);
        assert!(matches!(
            publication.observations()[0].inspect_process().unwrap(),
            Inspection::Present(_)
        ));
        assert_eq!(fs::read(&history).unwrap(), before);
        // The captured runtime address stays selected for a missing project;
        // another known location cannot silently switch to its state directory.
        let known_read = current
            .known_read_location(known.project_root(), &root.join("other-state"))
            .unwrap();
        assert_eq!(known_read.endpoint_directory(), known.endpoint_directory());
        host.shutdown().await.unwrap();
    });
}

#[test]
fn frozen_runtime_choice_and_history_cache_do_not_change_after_runtime_disappears() {
    let root = TestRuntimeRoot::new("history-frozen").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let layout = ResolvedLayout::resolve(LocationInputs {
        project_root: project,
        state_root: root.join("state"),
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            runtime_root: Some(runtime.clone()),
            cache_home: Some(root.join("cache")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    })
    .unwrap();
    fs::remove_dir(&runtime).unwrap();
    let missing = root.join("missing");
    let known = layout
        .known_read_location(&missing, &root.join("new-state"))
        .unwrap();
    assert_eq!(
        known.endpoint_directory(),
        runtime
            .join("runyte")
            .join(crate::workspace::workspace_id(&missing))
    );
    assert_eq!(
        layout.cache_root().unwrap(),
        Some(root.join("cache/runyte").as_path())
    );
    assert!(!runtime.exists());
    assert!(!root.join("new-state").exists());
    assert!(!root.join("cache").exists());
}

#[test]
fn snapshot_ready_observer_validates_exact_identity_and_cannot_retire_a_stale_record() {
    let root = TestRuntimeRoot::new("history-ready-proof").unwrap();
    let current = layout(&root, "project", "cache");
    let endpoint = current.publication_location().unwrap();
    let publication = endpoint.prepare(None).unwrap().publish().unwrap();
    let view = current.discovery_view(false).unwrap();
    let known = current.read_location();
    let directory = Directory::open_existing(current.endpoint_directory(), true).unwrap();
    let mut stale = publication.metadata().clone();
    stale.process.creation_time ^= 1;
    let stale_bytes = serde_json::to_vec(&stale).unwrap();
    directory
        .atomic_write(OsStr::new("endpoint.json"), &stale_bytes)
        .unwrap();
    let candidate = view.observe_snapshot_ready(&known).unwrap().unwrap();
    let Inspection::Stale(evidence) = candidate.inspect_process().unwrap() else {
        panic!("fixture should have reused identity")
    };
    assert!(evidence.remove_observed().is_err());
    assert_eq!(fs::read(endpoint.ready_record()).unwrap(), stale_bytes);
    drop(candidate);
    let other = layout(&root, "other", "cache");
    let prepared = other.publication_location().unwrap().prepare(None).unwrap();
    let foreign_bytes = serde_json::to_vec(prepared.metadata()).unwrap();
    drop(prepared);
    directory
        .atomic_write(OsStr::new("endpoint.json"), &foreign_bytes)
        .unwrap();
    let error = view.observe_snapshot_ready(&known).unwrap_err();
    assert!(error.to_string().contains("workspace identity mismatch"));
    assert_eq!(fs::read(endpoint.ready_record()).unwrap(), foreign_bytes);
    drop(publication);
}

#[test]
fn same_project_publications_stay_distinct_ambiguous_and_unnumbered() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-duplicate-live").unwrap();
        let left = layout(&root, "project", "cache-a");
        let right = ResolvedLayout::resolve(LocationInputs {
            project_root: left.project_root().into(),
            state_root: root.join("other-state"),
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                cache_home: Some(root.join("cache-b")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        let (_, mut a) = server(&left, "first");
        let (_, mut b) = server(&right, "second");
        let history = write_history(
            &left,
            &[remembered(
                left.project_root(),
                Some("history-name"),
                Some(8),
            )],
        );
        let before = fs::read(&history).unwrap();
        let (result, (), ()) = tokio::join!(
            snapshot_with_history(&left, Path::new(".runyte"), true),
            answer(&mut a, health()),
            answer(&mut b, health())
        );
        let mut result = result.unwrap();
        assert_eq!(result.entries().len(), 2);
        assert!(result.select(left.project_root(), None).is_err());
        assert!(result.select(Path::new(&a.metadata().id), None).is_err());
        let first = result
            .entries()
            .iter()
            .find(|entry| entry.row().name.as_deref() == Some("first"))
            .unwrap()
            .row()
            .selection();
        let second = result
            .entries()
            .iter()
            .find(|entry| entry.row().name.as_deref() == Some("second"))
            .unwrap()
            .row()
            .selection();
        assert_ne!(first, second);
        assert_eq!(first.project_root(), second.project_root());
        for (selection, expected) in [(&first, a.metadata()), (&second, b.metadata())] {
            let index = result.select_selection(selection).unwrap().unwrap();
            let HistoryTarget::Live { publication, .. } = result.target(index).unwrap() else {
                panic!("selected native row lost its live publication")
            };
            assert_eq!(publication.metadata(), expected);
        }
        for entry in result.entries() {
            assert_eq!(entry.row().number, None);
            assert_eq!(entry.row().last_active_unix_seconds, None);
        }
        let Some(HistoryTarget::Live { publication, .. }) =
            result.select(Path::new("second"), None).unwrap()
        else {
            panic!("exact name lost")
        };
        assert_eq!(publication.metadata(), b.metadata());
        assert!(
            result
                .select(Path::new("history-name"), None)
                .unwrap()
                .is_none()
        );
        assert_eq!(fs::read(&history).unwrap(), before);
        let duplicate = HistoryEntry {
            row: result.entries[0].row.clone(),
            live_index: result.entries[0].live_index,
        };
        result.entries.push(duplicate);
        assert!(
            result
                .select_selection(&result.entries[0].row.selection())
                .is_err()
        );
        drop(result);
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
        let (_, mut replacement) = server(&left, "replacement");
        let (later, ()) = tokio::join!(
            snapshot_with_history(&left, Path::new(".runyte"), true),
            answer(&mut replacement, health())
        );
        let later = later.unwrap();
        assert!(later.select_selection(&first).unwrap().is_none());
        assert!(later.select_selection(&second).unwrap().is_none());
        let current = later.entries()[0].row().selection();
        assert_ne!(current, first);
        assert_ne!(current, second);
        assert_eq!(later.select_selection(&current).unwrap(), Some(0));
        replacement.shutdown().await.unwrap();
    });
}

#[test]
fn hidden_inventory_only_host_never_inherits_local_history_name_or_digit() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-hidden").unwrap();
        let current = layout(&root, "current", "cache-a");
        let hidden = ResolvedLayout::resolve(LocationInputs {
            project_root: root.create_private_dir("hidden").unwrap(),
            state_root: root.join("foreign-state"),
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                cache_home: Some(root.join("cache-b")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        let endpoint = hidden.publication_location().unwrap();
        let mut host =
            crate::workspace::windows_transport::LocalServer::bind(endpoint.prepare(None).unwrap())
                .unwrap();
        let path = write_history(
            &current,
            &[remembered(hidden.project_root(), Some("local"), Some(6))],
        );
        let before = fs::read(&path).unwrap();
        let (result, ()) = tokio::join!(
            snapshot_with_history(&current, Path::new(".runyte"), true),
            answer(&mut host, health())
        );
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 1);
        let row = result.entries()[0].row();
        assert_eq!(row.name, None);
        assert_eq!(row.number, None);
        assert_eq!(row.last_active_unix_seconds, None);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!hidden.project_root().join(".runyte").exists());
        host.shutdown().await.unwrap();
    });
}

#[test]
fn malformed_history_and_ready_uncertainty_never_rewrite_or_prepare_storage() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-corrupt").unwrap();
        let current = layout(&root, "project", "cache");
        let path = write_history(&current, &[]);
        let cache = Directory::open_existing(path.parent().unwrap(), true).unwrap();
        cache
            .atomic_write(OsStr::new("workspaces.json"), b"invalid")
            .unwrap();
        assert!(
            snapshot_with_history(&current, Path::new(".runyte"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
        assert!(!path.with_extension("lock").exists());
        let known = layout(&root, "known", "cache");
        let path = write_history(&current, &[remembered(known.project_root(), None, None)]);
        let before = fs::read(&path).unwrap();
        Directory::open(known.endpoint_directory(), true)
            .unwrap()
            .atomic_write(OsStr::new("endpoint.json"), b"invalid")
            .unwrap();
        assert!(
            snapshot_with_history(&current, Path::new(".runyte"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("lock").exists());
    });
}

#[test]
fn all_256_remembered_locations_are_checked_without_truncation_or_new_paths() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-bound").unwrap();
        let current = layout(&root, "current", "cache");
        let canonical_root = root.path().canonicalize().unwrap();
        let entries = (0..RECENT_LIMIT)
            .map(|index| remembered(&canonical_root.join(format!("missing-{index}")), None, None))
            .collect::<Vec<_>>();
        let path = write_history(&current, &entries);
        let before = fs::read(&path).unwrap();
        let result = snapshot_with_history(&current, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert_eq!(result.remembered().len(), RECENT_LIMIT);
        assert_eq!(result.live().absent_projects().len(), RECENT_LIMIT + 1);
        assert!(result.entries().is_empty());
        assert_eq!(fs::read(&path).unwrap(), before);
        // A malformed ready record at the last remembered address must not be
        // missed by a truncated read-location list.
        let last = &entries[RECENT_LIMIT - 1].project_root;
        Directory::open(&last.join(".runyte/host"), true)
            .unwrap()
            .atomic_write(OsStr::new("endpoint.json"), b"invalid")
            .unwrap();
        let error = snapshot_with_history(&current, Path::new(".runyte"), false)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("expected value at line 1 column 1"),
            "{error:#}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    });
}

#[test]
fn captured_home_routing_keeps_alias_identity_through_history_and_initialization() {
    runtime().block_on(async {
        use crate::{
            project_root::{self, WorkspaceOrigin},
            workspace::{
                windows_control::ControlSnapshot, windows_endpoint::NameStore,
                windows_location::DiscoveryInputs,
            },
            workspace_state::StateLocation,
        };

        let root = TestRuntimeRoot::new("history-home-capture").unwrap();
        let home = root.create_private_dir("home").unwrap();
        let replacement_home = root.create_private_dir("other-home").unwrap();
        let state = root.create_private_dir("state").unwrap();
        let home_alias = root.join("home-alias");
        junction(&home_alias, &home);
        let scope = DiscoveryScope::resolve(DiscoveryInputs {
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                cache_home: Some(root.join("cache")),
                inventory_override: Some(root.join("inventory")),
                home_paths: vec![home_alias.clone()],
                user_state_root: Some(state.clone()),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        let safety = scope.capture_safety();
        // The raw home alias now resolves elsewhere. A recapture would change
        // its identity; ambient account paths cannot supply this injected home.
        fs::remove_dir(&home_alias).unwrap();
        junction(&home_alias, &replacement_home);
        assert!(
            project_root::discover_with_home_safety(
                &home,
                Path::new(".runyte"),
                &scope.capture_safety(),
            )
            .unwrap()
            .is_none()
        );
        let discovered =
            project_root::discover_with_home_safety(&home.join("."), Path::new(".runyte"), &safety)
                .unwrap()
                .unwrap();
        assert_eq!(discovered.root, home);
        assert_eq!(discovered.origin, WorkspaceOrigin::Home);
        let selected = project_root::resolve_state_location_with_safety(
            &discovered.root,
            Path::new(".runyte"),
            &safety,
        )
        .unwrap();
        assert_eq!(selected, StateLocation::out_of_tree(&home, &state));
        let current = scope
            .known_read_location(&discovered.root, selected.as_path())
            .unwrap();
        let layout = scope
            .initialize_layout_with_safety(&home.join("."), Path::new(".runyte"), &safety)
            .unwrap();
        assert_eq!(layout.project_root(), discovered.root);
        assert_eq!(layout.state_root(), selected.as_path());
        assert_eq!(layout.endpoint_directory(), current.endpoint_directory());

        let id = crate::workspace::workspace_id(&home);
        // Use the normal store to establish its stable lock before writing a
        // fixture name. History must read this exact out-of-tree store.
        assert_eq!(
            NameStore::open(layout.state_root())
                .unwrap()
                .load(&id)
                .unwrap(),
            None
        );
        Directory::open(layout.name_store_root(), true)
            .unwrap()
            .atomic_write(OsStr::new(&format!("{id}.json")), br#""captured-home""#)
            .unwrap();
        write_history(&layout, &[remembered(&home, None, None)]);
        let controls = ControlSnapshot::observe_at_with_safety(
            &scope,
            Some(&current),
            Path::new(".runyte"),
            false,
            &safety,
        )
        .await
        .unwrap();
        let Some(HistoryTarget::Stopped { row }) = controls
            .history()
            .select(Path::new("captured-home"), None)
            .unwrap()
        else {
            panic!("captured home state was not used by history observation")
        };
        assert_eq!(row.project_root, home);
        assert_eq!(row.id, id);
        assert!(!home.join(".runyte").exists());
        assert!(!replacement_home.join(".runyte").exists());
        fs::remove_dir(&home_alias).unwrap();
    });
}

fn junction(link: &std::path::Path, target: &std::path::Path) {
    use std::{
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        },
    };

    std::fs::create_dir(link).unwrap();
    let junction = std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ | GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(link)
        .unwrap();
    let target = crate::windows_fs::ordinary_working_directory(target).unwrap();
    let substitute: Vec<_> = std::ffi::OsString::from(format!(r"\??\{}", target.display()))
        .encode_wide()
        .collect();
    let mut data = Vec::new();
    data.extend_from_slice(&0xA0000003u32.to_le_bytes());
    data.extend_from_slice(&((8 + (substitute.len() + 2) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
    data.extend_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    for unit in substitute {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data.extend_from_slice(&[0; 4]);
    let mut returned = 0;
    assert_ne!(
        unsafe {
            windows_sys::Win32::System::IO::DeviceIoControl(
                junction.as_raw_handle(),
                0x000900A4,
                data.as_ptr().cast(),
                data.len() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
}
