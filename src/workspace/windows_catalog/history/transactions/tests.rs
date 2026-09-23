// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    private_storage::Directory,
    test_support::TestRuntimeRoot,
    workspace::{
        recent_history::{encode_recents, read_recents},
        windows_catalog::{
            snapshot_with_history,
            tests::{health, layout, runtime, server},
        },
        windows_location::{CapturedRoots, LocationInputs},
    },
};
use std::{ffi::OsStr, fs};

async fn answer(
    server: &mut crate::workspace::windows_transport::LocalServer,
    health: crate::protocol::HostResponse,
) {
    use crate::{
        protocol::{ClientRequest, FeatureGroup, HostResponse, VERSION},
        workspace::windows_transport::ServerEvent,
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut sender = None;
        loop {
            match server.recv().await.unwrap() {
                ServerEvent::Connected { responses, .. } => {
                    responses
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
                    sender = Some(responses);
                }
                ServerEvent::Request {
                    request: ClientRequest::Health,
                    ..
                } => {
                    sender.as_ref().unwrap().send(health).await.unwrap();
                    break;
                }
                // Several tests deliberately refresh the same native fixture.
                ServerEvent::Disconnected { .. } => {}
                event => panic!("unexpected fixture event: {event:?}"),
            }
        }
    })
    .await
    .unwrap();
}

fn entry(path: &Path, name: Option<&str>, number: Option<u8>) -> RecentEntry {
    RecentEntry::new(path.to_owned(), name.map(str::to_owned), number, Some(10))
}
fn path(layout: &ResolvedLayout) -> PathBuf {
    layout
        .cache_root()
        .unwrap()
        .unwrap()
        .join("workspaces.json")
}
fn write(layout: &ResolvedLayout, entries: &[RecentEntry]) {
    Directory::open(path(layout).parent().unwrap(), true)
        .unwrap()
        .atomic_write(
            OsStr::new("workspaces.json"),
            &encode_recents(entries).unwrap(),
        )
        .unwrap();
}
fn read(layout: &ResolvedLayout) -> Vec<RecentEntry> {
    read_recents(Some(&path(layout))).unwrap()
}
fn index(snapshot: &HistorySnapshot, project: &Path) -> usize {
    snapshot
        .entries()
        .iter()
        .position(|entry| entry.row().project_root == project)
        .unwrap()
}

#[test]
fn explicit_visits_use_exact_missing_identity_and_one_current_transaction() {
    let root = TestRuntimeRoot::new("history-exact-visits").unwrap();
    let current = layout(&root, "project", "cache");
    let project = current.project_root().to_owned();
    fs::remove_dir(&project).unwrap();
    let recorded = ensure_recorded(&current).unwrap().unwrap();
    assert_eq!(recorded.name, "project");
    assert_eq!(recorded.number, Some(1));
    let other_layout = layout(&root, "other", "cache");
    let other = other_layout.project_root().to_owned();
    remember(&other_layout).unwrap();
    ensure_recorded(&current).unwrap();
    assert_eq!(read(&current)[0].project_root, other);
    record_activity(&current, 80).unwrap();
    record_activity(&current, 20).unwrap();
    let rows = read(&current);
    assert_eq!(rows[0].project_root, project);
    assert_eq!(rows[0].last_active_unix_seconds, Some(80));
    assert_eq!(rows[1].project_root, other);
    assert!(!project.exists());
    assert!(!current.state_root().exists());
    // These APIs accept only the layout's already captured canonical identity;
    // callers cannot supply an ordinary spelling or another arbitrary path.
    assert_eq!(rows[0].project_root, current.project_root());
}

#[test]
fn remember_preserves_declined_digits_names_and_bounded_recency_tail() {
    let root = TestRuntimeRoot::new("history-recency").unwrap();
    let current = layout(&root, "project", "cache");
    let canonical = root.path().canonicalize().unwrap();
    let mut rows = (0..RECENT_LIMIT)
        .map(|n| {
            entry(
                &canonical.join(format!("old-{n}")),
                Some(&format!("saved-{n}")),
                None,
            )
        })
        .collect::<Vec<_>>();
    rows[12].number_declined = true;
    let target = rows[12].project_root.clone();
    write(&current, &rows);
    let target_layout = layout(&root, "old-12", "cache");
    let recorded = remember(&target_layout).unwrap().unwrap();
    assert_eq!(recorded.name, "saved-12");
    assert_eq!(recorded.number, None);
    assert!(read(&current)[0].number_declined);
    let new_layout = layout(&root, "new", "cache");
    let new = new_layout.project_root().to_owned();
    ensure_recorded(&new_layout).unwrap();
    let stored = read(&current);
    assert_eq!(stored.len(), RECENT_LIMIT);
    assert_eq!(stored[0].project_root, new);
    assert!(
        stored
            .iter()
            .all(|entry| entry.project_root != rows[RECENT_LIMIT - 1].project_root)
    );
    assert!(
        stored
            .iter()
            .find(|entry| entry.project_root == target)
            .unwrap()
            .number_declined
    );
}

#[test]
fn no_optional_cache_is_a_noop_and_malformed_cache_is_preserved() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-optional").unwrap();
        let project = root.create_private_dir("project").unwrap();
        let runtime_root = root.create_private_dir("runtime").unwrap();
        let no_cache = ResolvedLayout::resolve(LocationInputs {
            project_root: project,
            state_root: root.join("state"),
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                runtime_root: Some(runtime_root),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        assert!(remember(&no_cache).unwrap().is_none());
        assert!(ensure_recorded(&no_cache).unwrap().is_none());
        assert!(record_activity(&no_cache, 10).unwrap().is_none());
        let snapshot = snapshot_with_history(&no_cache, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert_eq!(snapshot.persist().unwrap(), 0);
        assert_eq!(snapshot.clear_stopped().unwrap(), 0);
        assert!(!no_cache.state_root().exists());
        assert!(!root.join("inventory").exists());
        let (_, mut host) = server(&no_cache, "uncached");
        let (live, ()) = tokio::join!(
            snapshot_with_history(&no_cache, Path::new(".runyte"), false),
            answer(&mut host, health())
        );
        assert!(
            live.unwrap()
                .set_number(0, Some(1))
                .unwrap_err()
                .to_string()
                .contains("available history cache")
        );
        host.shutdown().await.unwrap();

        let cached = layout(&root, "cached", "cache");
        let directory = Directory::open(path(&cached).parent().unwrap(), true).unwrap();
        directory
            .atomic_write(OsStr::new("workspaces.json"), b"corrupt")
            .unwrap();
        assert!(ensure_recorded(&cached).is_err());
        assert!(
            snapshot_with_history(&cached, Path::new(".runyte"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(path(&cached)).unwrap(), b"corrupt");
    });
}

#[test]
fn refresh_preserves_concurrent_fields_new_entries_and_missing_records() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-refresh-cas").unwrap();
        let a = layout(&root, "a", "cache");
        let b = layout(&root, "b", "cache");
        let missing = root.path().canonicalize().unwrap().join("missing");
        let mut old_b = entry(b.project_root(), None, Some(8));
        old_b.number_pinned = true;
        write(
            &a,
            &[
                entry(a.project_root(), Some("a-old"), Some(7)),
                old_b,
                entry(&missing, Some("missing"), Some(9)),
            ],
        );
        let snapshot = snapshot_with_history(&a, Path::new(".runyte"), false)
            .await
            .unwrap();
        let newcomer = root.path().canonicalize().unwrap().join("new");
        update_recents_if_changed(&path(&a), |entries| {
            entries[0].name = Some("a-new".into());
            entries[0].number = Some(6);
            entries[0].number_pinned = true;
            entries[0].last_active_unix_seconds = Some(100);
            entries.insert(0, entry(&newcomer, Some("new"), Some(5)));
            Ok(())
        })
        .unwrap();
        assert_eq!(snapshot.persist().unwrap(), 1);
        let rows = read(&a);
        assert_eq!(rows[0].project_root, newcomer);
        let updated = rows
            .iter()
            .find(|entry| entry.project_root == a.project_root())
            .unwrap();
        assert_eq!(updated.name.as_deref(), Some("a-new"));
        assert_eq!(updated.number, Some(6));
        assert_eq!(updated.last_active_unix_seconds, Some(100));
        let stopped = rows
            .iter()
            .find(|entry| entry.project_root == b.project_root())
            .unwrap();
        assert_eq!(stopped.name.as_deref(), Some("b"));
        assert_eq!(number_state(stopped), (None, false, false));
        assert_eq!(
            rows.iter()
                .find(|entry| entry.project_root == missing)
                .unwrap()
                .number,
            Some(9)
        );
        let before = fs::read(path(&a)).unwrap();
        assert_eq!(snapshot.persist().unwrap(), 0);
        assert_eq!(fs::read(path(&a)).unwrap(), before);
    });
}

#[test]
fn forget_and_clear_only_remove_unchanged_observed_stopped_rows() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-clear-cas").unwrap();
        let a = layout(&root, "a", "cache");
        let b = layout(&root, "b", "cache");
        let missing = root.path().canonicalize().unwrap().join("missing");
        write(
            &a,
            &[
                entry(a.project_root(), Some("a"), None),
                entry(b.project_root(), Some("b"), None),
                entry(&missing, None, None),
            ],
        );
        let old = snapshot_with_history(&a, Path::new(".runyte"), false)
            .await
            .unwrap();
        record_activity(&a, 90).unwrap();
        let new_layout = layout(&root, "new", "cache");
        let new = new_layout.project_root().to_owned();
        remember(&new_layout).unwrap();
        assert!(old.forget(index(&old, a.project_root())).is_err());
        assert_eq!(old.clear_stopped().unwrap(), 1);
        let rows = read(&a);
        assert!(
            rows.iter()
                .all(|entry| entry.project_root != b.project_root())
        );
        for kept in [a.project_root(), missing.as_path(), new.as_path()] {
            assert!(rows.iter().any(|entry| entry.project_root == kept));
        }
        let fresh = snapshot_with_history(&a, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert!(fresh.forget(index(&fresh, a.project_root())).unwrap());
        assert!(a.project_root().is_dir());
        assert!(fresh.forget(index(&fresh, a.project_root())).is_err());
    });
}

#[test]
fn native_live_number_swap_and_decline_require_current_scoped_records() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-number-cas").unwrap();
        let a = layout(&root, "a", "cache");
        let b = layout(&root, "b", "cache");
        write(
            &a,
            &[
                entry(a.project_root(), Some("a"), Some(1)),
                entry(b.project_root(), Some("b"), Some(2)),
            ],
        );
        let (_, mut ahost) = server(&a, "a");
        let (_, mut bhost) = server(&b, "b");
        let (snapshot, (), ()) = tokio::join!(
            snapshot_with_history(&a, Path::new(".runyte"), false),
            answer(&mut ahost, health()),
            answer(&mut bhost, health())
        );
        let snapshot = snapshot.unwrap();
        let target = index(&snapshot, a.project_root());
        assert!(snapshot.forget(target).is_err());
        assert!(snapshot.set_number(target, Some(0)).is_err());
        assert_eq!(
            snapshot.set_number(target, Some(2)).unwrap().as_deref(),
            Some(b.project_root())
        );
        let rows = read(&a);
        assert_eq!(rows[0].number, Some(2));
        assert!(rows[0].number_pinned);
        assert_eq!(rows[1].number, Some(1));
        assert!(snapshot.set_number(target, None).is_err());
        let (fresh, (), ()) = tokio::join!(
            snapshot_with_history(&a, Path::new(".runyte"), false),
            answer(&mut ahost, health()),
            answer(&mut bhost, health())
        );
        let fresh = fresh.unwrap();
        fresh
            .set_number(index(&fresh, a.project_root()), None)
            .unwrap();
        assert_eq!(number_state(&read(&a)[0]), (None, true, false));
        ahost.shutdown().await.unwrap();
        bhost.shutdown().await.unwrap();
    });
}

#[test]
fn refresh_does_not_duplicate_a_digit_or_name_claimed_after_observation() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-refresh-collision").unwrap();
        let a = layout(&root, "a", "cache");
        write(&a, &[entry(a.project_root(), Some("old"), Some(7))]);
        let (_, mut host) = server(&a, "host-name");
        let (snapshot, ()) = tokio::join!(
            snapshot_with_history(&a, Path::new(".runyte"), false),
            answer(&mut host, health())
        );
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot.entries()[0].row().number, Some(1));
        let new = root.path().canonicalize().unwrap().join("new");
        update_recents_if_changed(&path(&a), |entries| {
            entries.push(entry(&new, Some("host-name"), Some(1)));
            Ok(())
        })
        .unwrap();
        let before = fs::read(path(&a)).unwrap();
        assert_eq!(snapshot.persist().unwrap(), 0);
        assert_eq!(fs::read(path(&a)).unwrap(), before);
        assert!(snapshot.set_number(0, Some(1)).is_err());
        assert_eq!(fs::read(path(&a)).unwrap(), before);
        host.shutdown().await.unwrap();
    });
}

#[test]
fn hidden_and_duplicate_project_publications_cannot_mutate_local_history() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-hidden-write").unwrap();
        let a = layout(&root, "project", "cache-a");
        let b = ResolvedLayout::resolve(LocationInputs {
            project_root: a.project_root().into(),
            state_root: root.join("other-state"),
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                cache_home: Some(root.join("cache-b")),
                inventory_override: Some(root.join("inventory")),
                ..CapturedRoots::default()
            },
        })
        .unwrap();
        write(&a, &[entry(a.project_root(), Some("cached"), Some(8))]);
        let (_, mut hidden) = server(&b, "hidden");
        let (snapshot, ()) = tokio::join!(
            snapshot_with_history(&a, Path::new(".runyte"), true),
            answer(&mut hidden, health())
        );
        let snapshot = snapshot.unwrap();
        let before = fs::read(path(&a)).unwrap();
        assert_eq!(snapshot.persist().unwrap(), 0);
        assert!(snapshot.set_number(0, Some(1)).is_err());
        assert!(snapshot.forget(0).is_err());
        assert_eq!(snapshot.clear_stopped().unwrap(), 0);
        assert_eq!(fs::read(path(&a)).unwrap(), before);

        let (_, mut visible) = server(&a, "visible");
        let (duplicate, (), ()) = tokio::join!(
            snapshot_with_history(&a, Path::new(".runyte"), true),
            answer(&mut hidden, health()),
            answer(&mut visible, health())
        );
        let duplicate = duplicate.unwrap();
        assert_eq!(duplicate.entries().len(), 2);
        assert_eq!(duplicate.persist().unwrap(), 0);
        for n in 0..2 {
            assert!(duplicate.set_number(n, Some(1)).is_err());
        }
        assert_eq!(fs::read(path(&a)).unwrap(), before);
        hidden.shutdown().await.unwrap();
        visible.shutdown().await.unwrap();
    });
}

#[test]
fn hand_edited_duplicate_digits_refuse_an_unrelated_transaction_without_repair() {
    let root = TestRuntimeRoot::new("history-duplicate-digits").unwrap();
    let current = layout(&root, "project", "cache");
    let canonical = root.path().canonicalize().unwrap();
    let mut duplicate = entry(&canonical.join("second"), Some("second"), Some(3));
    duplicate.number_pinned = true;
    write(
        &current,
        &[
            entry(&canonical.join("first"), Some("first"), Some(3)),
            duplicate,
        ],
    );
    let before = fs::read(path(&current)).unwrap();
    // The compatibility reader still normalizes this old format in memory.
    assert_eq!(read(&current)[1].number, None);
    let error = ensure_recorded(&current).unwrap_err();
    assert!(error.to_string().contains("duplicate session numbers"));
    assert_eq!(fs::read(path(&current)).unwrap(), before);
}

#[test]
fn name_permutations_commit_as_one_batch_and_actions_require_a_fresh_snapshot() {
    runtime().block_on(async {
        let root = TestRuntimeRoot::new("history-name-permutation").unwrap();
        let a = layout(&root, "a", "cache");
        let b = layout(&root, "b", "cache");
        write(
            &a,
            &[
                entry(a.project_root(), Some("first"), Some(7)),
                entry(b.project_root(), Some("second"), Some(8)),
            ],
        );
        let (_, mut ahost) = server(&a, "second");
        let (_, mut bhost) = server(&b, "first");
        let (snapshot, (), ()) = tokio::join!(
            snapshot_with_history(&a, Path::new(".runyte"), false),
            answer(&mut ahost, health()),
            answer(&mut bhost, health())
        );
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot.persist().unwrap(), 2);
        let stored = read(&a);
        assert_eq!(stored[0].name.as_deref(), Some("second"));
        assert_eq!(stored[1].name.as_deref(), Some("first"));
        assert_eq!(stored[0].number, Some(1));
        assert_eq!(stored[1].number, Some(2));
        assert!(
            snapshot
                .set_number(index(&snapshot, a.project_root()), Some(2))
                .is_err()
        );
        let (fresh, (), ()) = tokio::join!(
            snapshot_with_history(&a, Path::new(".runyte"), false),
            answer(&mut ahost, health()),
            answer(&mut bhost, health())
        );
        let fresh = fresh.unwrap();
        assert_eq!(
            fresh
                .set_number(index(&fresh, a.project_root()), Some(2))
                .unwrap()
                .as_deref(),
            Some(b.project_root())
        );
        ahost.shutdown().await.unwrap();
        bhost.shutdown().await.unwrap();
    });
}
