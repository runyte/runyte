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
        let result = result.unwrap();
        assert_eq!(result.entries().len(), 2);
        assert!(result.select(left.project_root(), None).is_err());
        assert!(result.select(Path::new(&a.metadata().id), None).is_err());
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
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
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
