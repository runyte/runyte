// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::test_support::TestRuntimeRoot;

// Windows always uses explicitly owned sidecars. Unix also exercises that
// bootstrap path so its ordering and recovery have cross-platform coverage.
fn storage_policies() -> &'static [bool] {
    if cfg!(windows) {
        &[true]
    } else {
        &[false, true]
    }
}

fn route(workspace: &str, identity: &str) -> Route {
    Route {
        workspace: workspace.into(),
        identity: identity.into(),
    }
}
fn actor(store: &mut Store, n: u64) -> Actor {
    store
        .register(
            &route("workspace-a", "agent"),
            &format!("{n:064x}"),
            "codex",
            "storage test",
        )
        .unwrap()
}
fn key(n: u64) -> SendKey {
    SendKey {
        issued_ms: 1000,
        nonce: format!("{n:064x}"),
    }
}
fn location() -> Location {
    Location {
        directory: "/work/project".into(),
        ..Location::default()
    }
}
fn root() -> TestRuntimeRoot {
    TestRuntimeRoot::new("durable-channel").unwrap()
}
fn send(s: &mut Store, a: &Actor, c: &str, n: u64) -> u64 {
    s.post(a, c, &key(n), &format!("message {n}"), &location(), 1000)
        .unwrap()
}

#[test]
fn public_history_survives_restart_and_receipts_do_not_skip_unseen_messages() {
    let root = root();
    let mut store = Store::open(root.path()).unwrap();
    let a = actor(&mut store, 1);
    let b = actor(&mut store, 2);
    let c = store
        .create(&a, "public", ConversationKind::Public, &[])
        .unwrap();
    store.subscribe(&b, &c).unwrap();
    for n in 0..35 {
        send(&mut store, &a, &c, n);
    }
    let first = store.read(&b, &c, ReadMode::Unread, 10, 1000).unwrap();
    assert_eq!(first.messages.len(), 10);
    assert!(first.more);
    let last = store
        .read(&b, &c, ReadMode::History { before: None }, 5, 1000)
        .unwrap();
    store
        .acknowledge(&b, &c, last.receipt.as_ref().unwrap(), 1000)
        .unwrap();
    let replay = store.read(&b, &c, ReadMode::Unread, 10, 1000).unwrap();
    assert_eq!(first.receipt, replay.receipt);
    assert_eq!(replay.messages[0].text, "message 0");
    drop(store);
    let mut store = Store::open(root.path()).unwrap();
    let recovered = store.resume(&b.route, &format!("{:064x}", 2)).unwrap();
    assert_eq!(recovered.id(), b.id());
    store
        .acknowledge(&recovered, &c, first.receipt.as_ref().unwrap(), 1001)
        .unwrap();
    store
        .acknowledge(&recovered, &c, first.receipt.as_ref().unwrap(), 1001)
        .unwrap();
    let next = store.read(&b, &c, ReadMode::Unread, 200, 1001).unwrap();
    assert_eq!(next.messages.len(), 20);
    assert_eq!(next.messages[0].text, "message 10");
    let archive = store
        .read(&b, &c, ReadMode::History { before: None }, 200, 1001)
        .unwrap();
    assert_eq!(archive.messages.len(), 35);
    assert!(
        store
            .acknowledge(&a, &c, first.receipt.as_ref().unwrap(), 1001)
            .is_err()
    );
}

#[test]
fn late_subscription_keeps_initial_twenty_and_all_later_unread_messages() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    let b = actor(&mut s, 2);
    let c = s
        .create(&a, "public", ConversationKind::Public, &[])
        .unwrap();
    for n in 0..30 {
        send(&mut s, &a, &c, n);
    }
    s.subscribe(&b, &c).unwrap();
    for n in 30..65 {
        send(&mut s, &a, &c, n);
    }
    s.subscribe(&b, &c).unwrap();
    let first = s.read(&b, &c, ReadMode::Unread, 20, 1000).unwrap();
    assert_eq!(first.messages.first().unwrap().text, "message 10");
    assert_eq!(first.messages.last().unwrap().text, "message 29");
    assert!(first.more);
    let older = s
        .read(
            &b,
            &c,
            ReadMode::History {
                before: first.older_before,
            },
            20,
            1000,
        )
        .unwrap();
    assert_eq!(older.messages.len(), 10);
    s.acknowledge(&b, &c, older.receipt.as_ref().unwrap(), 1000)
        .unwrap();
    assert_eq!(
        s.read(&b, &c, ReadMode::Unread, 20, 1000).unwrap().receipt,
        first.receipt
    );
    s.acknowledge(&b, &c, first.receipt.as_ref().unwrap(), 1000)
        .unwrap();
    let rest = s.read(&b, &c, ReadMode::Unread, 200, 1000).unwrap();
    assert_eq!(rest.messages.len(), 35);
}

#[test]
fn fixed_group_deletes_only_after_every_recipient_and_keeps_send_tombstone() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    let b = actor(&mut s, 2);
    let c = actor(&mut s, 3);
    let outsider = actor(&mut s, 4);
    let group = s
        .create(
            &a,
            "group",
            ConversationKind::Fixed,
            &[b.id.clone(), c.id.clone()],
        )
        .unwrap();
    assert!(s.subscribe(&outsider, &group).is_err());
    assert!(
        s.read(
            &outsider,
            &group,
            ReadMode::History { before: None },
            20,
            1000
        )
        .is_err()
    );
    let seq = send(&mut s, &a, &group, 1);
    let rb = s.read(&b, &group, ReadMode::Unread, 20, 1000).unwrap();
    s.acknowledge(&b, &group, rb.receipt.as_ref().unwrap(), 1000)
        .unwrap();
    assert_eq!(
        s.read(&a, &group, ReadMode::History { before: None }, 20, 1000)
            .unwrap()
            .messages
            .len(),
        1
    );
    drop(s);
    let mut s = Store::open(root.path()).unwrap();
    let rc = s.read(&c, &group, ReadMode::Unread, 20, 1000).unwrap();
    s.acknowledge(&c, &group, rc.receipt.as_ref().unwrap(), 1000)
        .unwrap();
    assert!(
        s.read(&a, &group, ReadMode::History { before: None }, 20, 1000)
            .unwrap()
            .messages
            .is_empty()
    );
    assert_eq!(send(&mut s, &a, &group, 1), seq);
    assert!(
        s.post(&a, &group, &key(1), "different", &location(), 1000)
            .is_err()
    );
    assert!(
        s.post(
            &a,
            &group,
            &key(1),
            "message 1",
            &location(),
            1000 + RETRY_MS
        )
        .is_err()
    );
    s.acknowledge(&b, &group, rb.receipt.as_ref().unwrap(), 1000)
        .unwrap();
}

#[test]
fn recovery_and_cross_workspace_policy_require_independent_native_authorization() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    let c = s
        .create(&a, "local", ConversationKind::Public, &[])
        .unwrap();
    let other = route("workspace-b", "agent");
    assert!(s.resume(&other, &format!("{:064x}", 1)).is_err());
    assert!(
        s.register(&other, &format!("{:064x}", 1), "codex", "retry")
            .is_err()
    );
    assert!(s.resume(&a.route, &format!("{:064x}", 999)).is_err());
    s.bind_route(a.id(), &other).unwrap();
    let recovered = s.resume(&other, &format!("{:064x}", 1)).unwrap();
    assert!(s.read(&recovered, &c, ReadMode::Unread, 20, 1000).is_err());
    let rev = s
        .replace_policy(&c, 1, &[a.route.clone(), other.clone()])
        .unwrap();
    send(&mut s, &recovered, &c, 1);
    assert!(s.replace_policy(&c, 1, &[]).is_err());
    s.replace_policy(&c, rev, std::slice::from_ref(&a.route))
        .unwrap();
    assert!(
        s.post(&recovered, &c, &key(2), "denied", &location(), 1000)
            .is_err()
    );
    assert!(s.read(&recovered, &c, ReadMode::Unread, 20, 1000).is_err());
    let different = s
        .register(
            &route("workspace-a", "claude"),
            &format!("{:064x}", 2),
            "claude",
            "test",
        )
        .unwrap();
    let local = s
        .create(&a, "all local identities", ConversationKind::Public, &[])
        .unwrap();
    s.subscribe(&different, &local).unwrap();
    assert!(s.read(&different, &c, ReadMode::Unread, 20, 1000).is_err());
    let unknown = s
        .read(&different, "missing", ReadMode::Unread, 20, 1000)
        .err()
        .unwrap()
        .to_string();
    let denied = s
        .read(&different, &c, ReadMode::Unread, 20, 1000)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(unknown, denied);
}

#[test]
fn concurrent_stores_deduplicate_sends_and_enforce_current_policy() {
    let root = root();
    let mut one = Store::open(root.path()).unwrap();
    let a = actor(&mut one, 1);
    let b = actor(&mut one, 2);
    let c = one
        .create(
            &a,
            "dm",
            ConversationKind::Fixed,
            std::slice::from_ref(&b.id),
        )
        .unwrap();
    let mut two = Store::open(root.path()).unwrap();
    let seq = send(&mut one, &a, &c, 1);
    assert_eq!(send(&mut two, &a, &c, 1), seq);
    let read = two.read(&b, &c, ReadMode::Unread, 20, 1000).unwrap();
    one.replace_policy(&c, 1, &[]).unwrap();
    assert!(
        two.acknowledge(&b, &c, read.receipt.as_ref().unwrap(), 1000)
            .is_err()
    );
    one.replace_policy(&c, 2, std::slice::from_ref(&a.route))
        .unwrap();
    two.acknowledge(&b, &c, read.receipt.as_ref().unwrap(), 1000)
        .unwrap();
    assert!(
        one.read(&a, &c, ReadMode::History { before: None }, 20, 1000)
            .unwrap()
            .messages
            .is_empty()
    );
}

#[test]
fn expired_receipts_leave_messages_replayable_and_operator_retirement_is_explicit() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    let b = actor(&mut s, 2);
    let c = s
        .create(
            &a,
            "dm",
            ConversationKind::Fixed,
            std::slice::from_ref(&b.id),
        )
        .unwrap();
    send(&mut s, &a, &c, 1);
    let read = s.read(&b, &c, ReadMode::Unread, 20, 1000).unwrap();
    assert!(
        s.acknowledge(&b, &c, read.receipt.as_ref().unwrap(), 1000 + RECEIPT_MS)
            .is_err()
    );
    let retry = s
        .read(&b, &c, ReadMode::Unread, 20, 1000 + RECEIPT_MS)
        .unwrap();
    assert_eq!(retry.messages.len(), 1);
    assert_ne!(read.receipt, retry.receipt);
    assert!(s.retire(&c, 2).is_err());
    s.retire(&c, 1).unwrap();
    assert!(s.read(&b, &c, ReadMode::Unread, 20, 1000).is_err());
    let count: i64 =
        s.db.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn refusal_preserves_unknown_corrupt_and_future_databases() {
    for mode in ["unknown", "future", "corrupt"] {
        let root = root();
        let dir = Directory::open(root.path(), true).unwrap();
        let mut f = dir.create_new(OsStr::new(FILE)).unwrap();
        if mode == "corrupt" {
            use std::io::Write;
            f.write_all(b"not sqlite").unwrap();
        }
        // The private Windows creation handle includes DELETE access. SQLite
        // does not share DELETE, so close our writer before opening its handle.
        drop(f);
        if mode != "corrupt" {
            let c = Connection::open(root.path().join(FILE)).unwrap();
            if mode == "unknown" {
                c.execute_batch("CREATE TABLE unrelated(value TEXT);")
                    .unwrap();
            } else {
                c.pragma_update(None, "user_version", 99).unwrap();
            }
        }
        let before = std::fs::read(root.path().join(FILE)).unwrap();
        for &private in storage_policies() {
            assert!(Store::open_with_private_sidecars(root.path(), private).is_err());
            assert_eq!(before, std::fs::read(root.path().join(FILE)).unwrap());
        }
    }
}

#[test]
fn bounded_reads_and_busy_writers_do_not_acknowledge_or_accept_failed_sends() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    let b = actor(&mut s, 2);
    let c = s
        .create(&a, "public", ConversationKind::Public, &[])
        .unwrap();
    s.subscribe(&b, &c).unwrap();
    let text = "\\".repeat(16 * 1024);
    for n in 0..20 {
        s.post(&a, &c, &key(n), &text, &location(), 1000).unwrap();
    }
    let p = s.read(&b, &c, ReadMode::Unread, 200, 1000).unwrap();
    assert!(p.more);
    assert!(p.messages.len() < 20);
    let hold = Connection::open(root.path().join(FILE)).unwrap();
    hold.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    assert!(
        s.post(&a, &c, &key(100), "blocked", &location(), 1000)
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    hold.execute_batch("ROLLBACK").unwrap();
    send(&mut s, &a, &c, 100);
    let count: i64 =
        s.db.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 21);
}

#[cfg(unix)]
#[test]
fn storage_rejects_links_and_preserves_private_database_and_sidecars() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let _ = actor(&mut s, 1);
    for name in [
        FILE,
        "agent-channels.sqlite3-wal",
        "agent-channels.sqlite3-shm",
    ] {
        assert_eq!(
            std::fs::metadata(root.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
    drop(s);
    let other = root.path().join("elsewhere");
    std::fs::write(&other, b"unchanged").unwrap();
    let journal = root.path().join("agent-channels.sqlite3-journal");
    symlink(&other, &journal).unwrap();
    assert!(Store::open(root.path()).is_err());
    assert_eq!(std::fs::read(&other).unwrap(), b"unchanged");
}

#[test]
fn input_validation_and_failed_groups_leave_no_partial_rows() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    assert!(
        s.register(&route("", "agent"), &format!("{:064x}", 2), "codex", "task")
            .is_err()
    );
    assert!(s.register(&a.route, "bad", "codex", "task").is_err());
    assert!(s.create(&a, "one", ConversationKind::Fixed, &[]).is_err());
    assert!(
        s.create(&a, "unknown", ConversationKind::Fixed, &["missing".into()])
            .is_err()
    );
    assert!(
        s.create(
            &a,
            "public",
            ConversationKind::Public,
            std::slice::from_ref(&a.id)
        )
        .is_err()
    );
    let count: i64 =
        s.db.query_row("SELECT count(*) FROM channels", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 0);
    let c = s
        .create(&a, "valid", ConversationKind::Public, &[])
        .unwrap();
    assert!(s.post(&a, &c, &key(1), "\0", &location(), 1000).is_err());
    assert!(
        s.post(&a, &c, &key(1), "ok", &Location::default(), 1000)
            .is_err()
    );
    assert!(s.read(&a, &c, ReadMode::Unread, 0, 1000).is_err());
    assert!(
        s.read(
            &a,
            &c,
            ReadMode::History {
                before: Some(u64::MAX)
            },
            20,
            1000
        )
        .is_err()
    );
}

#[test]
fn bounded_expiry_progress_survives_quota_refusal_and_renews_matching_receipt() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    let b = actor(&mut s, 2);
    let c = s
        .create(&a, "public", ConversationKind::Public, &[])
        .unwrap();
    s.subscribe(&b, &c).unwrap();
    send(&mut s, &a, &c, 1);
    // Put the relevant expired receipt after more than one maintenance batch.
    for n in 0..300 {
        s.db.execute(
            "INSERT INTO receipts VALUES (?1,?2,?3,?1,1,0)",
            params![format!("old-{n}"), a.id, c],
        )
        .unwrap();
    }
    let digest = sha256_hex(serde_json::to_string(&vec![1u64]).unwrap().as_bytes());
    s.db.execute(
        "INSERT INTO receipts VALUES ('old-matching',?1,?2,?3,2,0)",
        params![b.id, c, digest],
    )
    .unwrap();
    for n in 0..MAX_RECEIPTS {
        s.db.execute(
            "INSERT INTO receipts VALUES (?1,?2,?3,?1,999999,0)",
            params![format!("live-{n}"), b.id, c],
        )
        .unwrap();
    }
    assert!(s.read(&b, &c, ReadMode::Unread, 20, 1000).is_err());
    let left: i64 =
        s.db.query_row(
            "SELECT count(*) FROM receipts WHERE expires<=1000",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 45); // Quota error did not roll back bounded cleanup.
    s.db.execute("DELETE FROM receipts WHERE id LIKE 'live-%'", [])
        .unwrap();
    let page = s.read(&b, &c, ReadMode::Unread, 20, 1000).unwrap();
    assert_ne!(page.receipt.as_deref(), Some("old-matching"));
    s.acknowledge(&b, &c, page.receipt.as_ref().unwrap(), 1000)
        .unwrap();
}

#[test]
fn simultaneous_first_open_and_send_are_transactional() {
    for &private in storage_policies() {
        let root = root();
        let child = root.join("new-store");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = child.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Store::open_with_private_sidecars(&path, private)
                })
            })
            .collect();
        for handle in handles {
            match handle.join().unwrap() {
                Ok(_) => (),
                Err(error) => {
                    // SQLite lock contention is a supported bounded refusal; a
                    // filesystem creation race or invalid schema is not.
                    assert!(
                        matches!(error.downcast_ref::<rusqlite::Error>(), Some(rusqlite::Error::SqliteFailure(code,_)) if code.code==rusqlite::ErrorCode::DatabaseBusy),
                        "{error:#}"
                    );
                }
            }
        }
        let mut s = Store::open_with_private_sidecars(&child, private).unwrap();
        let a = actor(&mut s, 1);
        let c = s.create(&a, "race", ConversationKind::Public, &[]).unwrap();
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = child.clone();
                let a = a.clone();
                let c = c.clone();
                let barrier = barrier.clone();
                // Open before spawning: a bounded admission refusal must not strand
                // another worker at the send barrier.
                let mut s = Store::open_with_private_sidecars(&path, private).unwrap();
                std::thread::spawn(move || {
                    barrier.wait();
                    send(&mut s, &a, &c, 1)
                })
            })
            .collect();
        let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(ids[0], ids[1]);
        let count: i64 =
            s.db.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
                .unwrap();
        assert_eq!(count, 1);
    }
}

#[test]
fn full_database_refuses_send_without_partial_acceptance() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let a = actor(&mut s, 1);
    let c = s.create(&a, "full", ConversationKind::Public, &[]).unwrap();
    let pages: i64 =
        s.db.pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
    s.db.pragma_update(None, "max_page_count", pages).unwrap();
    let result = s.post(&a, &c, &key(1), &"x".repeat(16 * 1024), &location(), 1000);
    assert!(result.is_err());
    let messages: i64 =
        s.db.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
    let sends: i64 =
        s.db.query_row("SELECT count(*) FROM sends", [], |r| r.get(0))
            .unwrap();
    assert_eq!((messages, sends), (0, 0));
    s.db.pragma_update(None, "max_page_count", 65536).unwrap();
    send(&mut s, &a, &c, 1);
}

#[test]
fn ten_independent_participants_keep_receipts_until_each_acknowledges() {
    let root = root();
    let mut s = Store::open(root.path()).unwrap();
    let agents: Vec<_> = (1..=10).map(|n| actor(&mut s, n)).collect();
    let members: Vec<_> = agents[1..].iter().map(|a| a.id.clone()).collect();
    let c = s
        .create(&agents[0], "ten", ConversationKind::Fixed, &members)
        .unwrap();
    send(&mut s, &agents[0], &c, 1);
    for a in &agents[1..] {
        let page = s.read(a, &c, ReadMode::Unread, 20, 1000).unwrap();
        assert_eq!(page.messages.len(), 1);
        s.acknowledge(a, &c, page.receipt.as_ref().unwrap(), 1000)
            .unwrap();
    }
    assert!(
        s.read(&agents[0], &c, ReadMode::History { before: None }, 20, 1000)
            .unwrap()
            .messages
            .is_empty()
    );
}

#[test]
fn private_sidecars_survive_bootstrap_live_connections_and_last_close() {
    let root = root();
    let mut one = Store::open_with_private_sidecars(root.path(), true).unwrap();
    let a = actor(&mut one, 1);
    let mut two = Store::open_with_private_sidecars(root.path(), true).unwrap();
    assert_eq!(actor(&mut two, 1).id(), a.id());
    drop(one);
    drop(two);
    let directory = Directory::open_existing(root.path(), true).unwrap();
    for name in ["agent-channels.sqlite3-wal", "agent-channels.sqlite3-shm"] {
        directory.open_read(OsStr::new(name)).unwrap();
    }
    let mut reopened = Store::open_with_private_sidecars(root.path(), true).unwrap();
    assert_eq!(actor(&mut reopened, 1).id(), a.id());
    assert_eq!(
        reopened
            .db
            .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
}

#[test]
fn private_bootstrap_recovers_an_empty_database_with_an_abandoned_journal() {
    use std::io::Write;
    for abandoned in [b"".as_slice(), b"interrupted initial journal".as_slice()] {
        let root = root();
        let directory = Directory::open(root.path(), true).unwrap();
        bootstrap::ensure_file(&directory, FILE).unwrap();
        let mut journal = directory
            .create_new(OsStr::new("agent-channels.sqlite3-journal"))
            .unwrap();
        journal.write_all(abandoned).unwrap();
        drop(journal);
        let mut store = Store::open_with_private_sidecars(root.path(), true).unwrap();
        let identity = actor(&mut store, 1).id().to_owned();
        drop(store);
        let mut store = Store::open_with_private_sidecars(root.path(), true).unwrap();
        assert_eq!(actor(&mut store, 1).id(), identity);
    }
}

#[test]
fn private_bootstrap_contention_is_a_bounded_sqlite_busy_refusal() {
    let root = root();
    let directory = Directory::open(root.path(), true).unwrap();
    let guard = bootstrap::acquire(&directory).unwrap();
    let error = Store::open_with_private_sidecars(root.path(), true)
        .err()
        .unwrap();
    assert!(matches!(error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(code, _))
        if code.code == rusqlite::ErrorCode::DatabaseBusy));
    drop(guard);
    Store::open_with_private_sidecars(root.path(), true).unwrap();
}
