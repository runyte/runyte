// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    private_storage::Directory,
    test_support::TestRuntimeRoot,
    workspace::{
        recent_history::encode_recents,
        windows_catalog::tests::{answer, health, layout, runtime, server},
        windows_location::{CapturedRoots, LocationInputs},
        windows_transport::LocalServer,
    },
};
use std::{ffi::OsStr, fs};

fn history(layout: &ResolvedLayout, entries: &[RecentEntry]) -> PathBuf {
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
fn recent(layout: &ResolvedLayout, name: Option<&str>) -> RecentEntry {
    RecentEntry::new(
        layout.project_root().into(),
        name.map(str::to_owned),
        None,
        Some(42),
    )
}
fn name(layout: &ResolvedLayout, value: &str) {
    let registries =
        crate::workspace::windows_endpoint::RegistrySet::open(layout.namespace_roots()).unwrap();
    NameStore::open(layout.state_root())
        .unwrap()
        .store_if_absent(
            &registries,
            &crate::workspace::workspace_id(layout.project_root()),
            value,
        )
        .unwrap();
    assert_eq!(
        NameStore::read_existing(
            layout.state_root(),
            &crate::workspace::workspace_id(layout.project_root())
        )
        .unwrap()
        .as_deref(),
        Some(value)
    );
}

#[test]
fn authoritative_stopped_names_reserve_defaults_without_changing_the_cache_baseline() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("stored-default-order").unwrap();
        let default = layout(&root, "chosen", "cache");
        let explicit = layout(&root, "explicit", "cache");
        name(&explicit, "chosen");
        let originals = [recent(&default, None), recent(&explicit, Some("cached"))];
        let path = history(&default, &originals);
        let bytes = fs::read(&path).unwrap();
        let result = snapshot_with_history(&default, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert_eq!(result.remembered(), originals);
        let explicit_row = result
            .entries()
            .iter()
            .find(|entry| entry.row().project_root == explicit.project_root())
            .unwrap();
        assert_eq!(explicit_row.row().name.as_deref(), Some("chosen"));
        let default_row = result
            .entries()
            .iter()
            .find(|entry| entry.row().project_root == default.project_root())
            .unwrap();
        assert_ne!(default_row.row().name.as_deref(), Some("chosen"));
        assert!(
            default_row
                .row()
                .name
                .as_ref()
                .unwrap()
                .starts_with("chosen")
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!path.with_extension("lock").exists());
        assert!(!default.state_root().exists());
    });
}

#[test]
fn configured_live_names_reserve_stopped_defaults_even_without_remembered_live_history() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("live-default-order").unwrap();
        let stopped = layout(&root, "reserved", "cache");
        let live = layout(&root, "live", "cache");
        let (_, mut host) = server(&live, "reserved");
        history(&stopped, &[recent(&stopped, None)]);
        let (result, ()) = tokio::join!(
            snapshot_with_history(&stopped, Path::new(".runyte"), false),
            answer(&mut host, health())
        );
        let result = result.unwrap();
        assert_eq!(
            result
                .entries()
                .iter()
                .find(|entry| entry.row().running)
                .unwrap()
                .row()
                .name
                .as_deref(),
            Some("reserved")
        );
        assert_ne!(
            result
                .entries()
                .iter()
                .find(|entry| !entry.row().running)
                .unwrap()
                .row()
                .name
                .as_deref(),
            Some("reserved")
        );
        host.shutdown().await.unwrap();
    });
}

#[test]
fn live_metadata_including_no_name_ignores_stored_or_cached_local_names() {
    runtime().block_on(async {
        for hidden in [false, true] {
            let root = TestRuntimeRoot::new("live-name-authority").unwrap();
            let local = layout(&root, "project", "cache");
            let actual = if hidden {
                ResolvedLayout::resolve(LocationInputs {
                    project_root: local.project_root().into(),
                    state_root: root.join("hidden-state"),
                    reserved_user_roots: vec![root.join("config")],
                    roots: CapturedRoots {
                        cache_home: Some(root.join("hidden-cache")),
                        inventory_override: Some(root.join("inventory")),
                        ..CapturedRoots::default()
                    },
                })
                .unwrap()
            } else {
                local.clone()
            };
            let endpoint = actual.publication_location().unwrap();
            let mut host = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
            let names = NameStore::open(local.state_root()).unwrap();
            // A malformed local name is irrelevant while exact live authority
            // exists, including a foreign publication known only by inventory.
            names.load(&host.metadata().id).unwrap();
            Directory::open(local.name_store_root(), true)
                .unwrap()
                .atomic_write(
                    OsStr::new(&format!("{}.json", host.metadata().id)),
                    b"invalid",
                )
                .unwrap();
            history(&local, &[recent(&local, Some("cached"))]);
            let (result, ()) = tokio::join!(
                snapshot_with_history(&local, Path::new(".runyte"), hidden),
                answer(&mut host, health())
            );
            let result = result.unwrap();
            assert_eq!(result.entries().len(), 1);
            assert_eq!(result.entries()[0].row().name, None);
            assert!(result.stopped_name_editor(0).is_err());
            if hidden {
                assert!(
                    result.live().entries()[0]
                        .observations()
                        .iter()
                        .all(|item| item.origin() == CandidateOrigin::OwnerInventory)
                );
            }
            host.shutdown().await.unwrap();
        }
    });
}

#[test]
fn malformed_stopped_authority_refuses_a_complete_snapshot_without_rewriting_history() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("malformed-stopped-name").unwrap();
        let local = layout(&root, "project", "cache");
        name(&local, "stored");
        let id = crate::workspace::workspace_id(local.project_root());
        Directory::open(local.name_store_root(), true)
            .unwrap()
            .atomic_write(OsStr::new(&format!("{id}.json")), b"invalid")
            .unwrap();
        let path = history(&local, &[recent(&local, Some("cached"))]);
        let before = fs::read(&path).unwrap();
        assert!(
            snapshot_with_history(&local, Path::new(".runyte"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("lock").exists());
    });
}

#[test]
fn stopped_selection_checks_captured_authority_and_forgetting_never_deletes_it() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("selected-stopped-name").unwrap();
        let local = layout(&root, "project", "cache");
        history(&local, &[recent(&local, Some("cached"))]);
        let result = snapshot_with_history(&local, Path::new(".runyte"), false)
            .await
            .unwrap();
        let mut first = result.stopped_name_editor(0).unwrap();
        let mut stale = result.stopped_name_editor(0).unwrap();
        first.rename("explicit").unwrap();
        assert!(stale.rename("obsolete").is_err());
        let current = snapshot_with_history(&local, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert_eq!(current.entries()[0].row().name.as_deref(), Some("explicit"));
        current.forget(0).unwrap();
        assert!(
            read_recents(current.history_path.as_deref())
                .unwrap()
                .is_empty()
        );
        let names = NameStore::open(local.state_root()).unwrap();
        let prepared = local
            .publication_location()
            .unwrap()
            .prepare_named(&names, None)
            .unwrap();
        assert_eq!(prepared.metadata().name.as_deref(), Some("explicit"));
    });
}
