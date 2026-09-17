// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Darwin's default TMPDIR can itself exceed the Unix-socket path
        // budget; the system /tmp alias resolves to /private/tmp.
        let root = Path::new("/tmp").join(format!(
            "ryctx-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("project")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn store(&self) -> Storage {
        Storage::new(self.0.join("store")).unwrap()
    }
    fn project(&self) -> PathBuf {
        self.0.join("project")
    }
    fn registration(&self, store: &Storage, mode: HostMode) -> Registration {
        let host_incarnation = random_token().unwrap();
        let root = self.project();
        Registration {
            workspace_id: crate::workspace::identity::workspace_id(&root),
            root,
            endpoint: store.socket_path(&host_incarnation).unwrap(),
            host_incarnation,
            mode,
            pid: std::process::id(),
            environment: "a".repeat(64),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn identities_are_stable_distinct_private_and_redacted() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let first = store.identity("agent").unwrap();
    assert_eq!(
        first.credential(),
        store.identity("agent").unwrap().credential()
    );
    let other = store.identity("claude").unwrap();
    assert_ne!(first.credential(), other.credential());
    assert!(valid_hex(first.credential()));
    assert_eq!(first.name(), "agent");
    assert!(!format!("{first:?}").contains(first.credential()));
    assert_eq!(store.identities().unwrap().len(), 2);
    assert!(store.load_identity("missing").unwrap().is_none());
    assert!(store.identity("../escape").is_err());
    assert!(store.identity("").is_err());
    assert!(store.identity(&"a".repeat(65)).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(store.root())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(store.root().join("identity.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn remembered_grants_are_exact_per_workspace_and_identity_and_revocable() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let first = store.identity("agent").unwrap();
    let other = store.identity("other").unwrap();
    let scopes = BTreeSet::from([
        Scope::TerminalRead,
        Scope::EditorContextRead,
        Scope::BufferEdit,
    ]);
    store
        .grant(&fixture.project(), &first, scopes.clone())
        .unwrap();
    assert_eq!(
        store.scopes(&fixture.project().join("."), &first).unwrap(),
        scopes
    );
    assert!(store.scopes(&fixture.project(), &other).unwrap().is_empty());
    assert!(
        store
            .scopes(&fixture.0.join("other"), &first)
            .unwrap()
            .is_empty()
    );
    store.revoke(&fixture.project(), &first).unwrap();
    store.revoke(&fixture.project(), &first).unwrap();
    assert!(store.scopes(&fixture.project(), &first).unwrap().is_empty());
    assert!(store.grant(&fixture.0, &first, scopes).is_err());
}

#[test]
fn discovery_separates_incarnations_modes_and_environments() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let first = fixture.registration(&store, HostMode::Persistent);
    let mut second = fixture.registration(&store, HostMode::Standalone);
    second.environment = "b".repeat(64);
    store.register(&first).unwrap();
    store.register(&second).unwrap();
    assert_eq!(
        store.discover(&first.environment, false).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(store.discover(&first.environment, true).unwrap().len(), 2);
    store.unregister(&first.host_incarnation).unwrap();
    assert_eq!(
        store.discover(&first.environment, true).unwrap(),
        vec![second]
    );
    store.unregister(&first.host_incarnation).unwrap();
    assert!(store.discover("invalid", false).is_err());
    assert!(store.unregister("../outside").is_err());
}

#[test]
fn malformed_oversized_and_mismatched_records_fail_closed() {
    let fixture = Fixture::new();
    let store = fixture.store();
    std::fs::write(store.root().join("identity.json"), b"not json").unwrap();
    assert!(store.identity("agent").is_err());
    std::fs::write(
        store.root().join("identity.json"),
        vec![b' '; RECORD_LIMIT + 1],
    )
    .unwrap();
    assert!(store.load_identity("agent").is_err());
    store.remove("identity.json").unwrap();
    let identity = store.identity("agent").unwrap();
    let root = path_bytes(&fixture.project());
    let name = grant_file(&root, &identity.fingerprint());
    store
        .write(
            &name,
            &Grant {
                root: b"other".to_vec(),
                identity: identity.fingerprint(),
                scopes: BTreeSet::new(),
            },
        )
        .unwrap();
    assert!(store.scopes(&fixture.project(), &identity).is_err());
    store.write("identity-wrong.json", &identity).unwrap();
    assert!(store.load_identity("wrong").is_err());
    std::fs::write(store.root().join("host-bad.json"), "{}").unwrap();
    assert!(store.discover(&"a".repeat(64), true).unwrap().is_empty());
    assert!(
        store
            .write("too-large.json", &"a".repeat(RECORD_LIMIT))
            .is_err()
    );
}

#[test]
fn registration_rejects_wrong_paths_ids_and_incarnations() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut registration = fixture.registration(&store, HostMode::Standalone);
    let valid = registration.clone();
    registration.endpoint = fixture.0.join("other.sock");
    assert!(store.register(&registration).is_err());
    registration = valid.clone();
    registration.workspace_id = "wrong".into();
    assert!(store.register(&registration).is_err());
    registration = valid.clone();
    registration.root = registration.root.join(".");
    assert!(store.register(&registration).is_err());
    registration = valid.clone();
    registration.pid = 0;
    assert!(store.register(&registration).is_err());
    registration = valid.clone();
    registration.environment = "wrong".into();
    assert!(store.register(&registration).is_err());
    registration = valid.clone();
    registration.host_incarnation = "wrong".into();
    assert!(store.register(&registration).is_err());
    store.write("host-wrong-name.json", &valid).unwrap();
    assert!(store.discover(&valid.environment, true).unwrap().is_empty());
    assert!(store.socket_path("../outside").is_err());
    let long = Storage::new(fixture.0.join("x".repeat(100))).unwrap();
    assert!(long.socket_path(&valid.host_incarnation).is_err());
    assert!(Storage::new(PathBuf::from("relative")).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_hardlinks_and_fifos_never_supply_records() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let store = fixture.store();
    let identity = store.identity("agent").unwrap();
    symlink(store.root(), fixture.0.join("link")).unwrap();
    assert!(Storage::new(fixture.0.join("link")).is_err());
    symlink(
        store.root().join("identity.json"),
        store.root().join("identity-alias.json"),
    )
    .unwrap();
    assert!(store.load_identity("alias").is_err());
    std::fs::hard_link(
        store.root().join("identity.json"),
        fixture.0.join("hardlink"),
    )
    .unwrap();
    assert!(store.load_identity("agent").is_err());
    std::fs::remove_file(fixture.0.join("hardlink")).unwrap();
    let fifo =
        std::ffi::CString::new(path_bytes(&store.root().join("identity-fifo.json"))).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(store.load_identity("fifo").is_err());
    let alias = fixture.0.join("project-alias");
    symlink(fixture.project(), &alias).unwrap();
    store
        .grant(&alias, &identity, BTreeSet::from([Scope::TerminalRead]))
        .unwrap();
    assert_eq!(
        store.scopes(&fixture.project(), &identity).unwrap(),
        BTreeSet::from([Scope::TerminalRead])
    );
}

#[test]
fn concurrent_identity_creation_has_one_winner() {
    let fixture = Fixture::new();
    let root = fixture.store().root().to_owned();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let root = root.clone();
            std::thread::spawn(move || {
                Storage::new(root)
                    .unwrap()
                    .identity("agent")
                    .unwrap()
                    .credential()
                    .to_owned()
            })
        })
        .collect();
    let identities: BTreeSet<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(identities.len(), 1);
}

#[test]
fn invalid_write_scope_dependencies_are_rejected() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let identity = store.identity("agent").unwrap();
    for scope in [Scope::BufferEdit, Scope::TerminalPropose] {
        assert!(
            store
                .grant(&fixture.project(), &identity, BTreeSet::from([scope]))
                .is_err()
        );
    }
    let root = path_bytes(&fixture.project());
    store
        .write(
            &grant_file(&root, &identity.fingerprint()),
            &Grant {
                root,
                identity: identity.fingerprint(),
                scopes: BTreeSet::from([Scope::BufferEdit]),
            },
        )
        .unwrap();
    assert!(store.scopes(&fixture.project(), &identity).is_err());
}

#[test]
fn identities_are_bounded_and_ignore_corrupt_neighbors() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for number in 0..32 {
        store.identity(&format!("bridge-{number}")).unwrap();
    }
    std::fs::write(store.root().join("identity-broken.json"), "invalid").unwrap();
    assert_eq!(store.identities().unwrap().len(), 32);
    assert!(store.identity("overflow").is_err());
    let extra = Identity {
        name: "overflow".into(),
        credential: random_token().unwrap(),
    };
    store.write("identity-overflow.json", &extra).unwrap();
    assert!(store.identities().is_err());
}

#[cfg(unix)]
#[test]
fn permissive_owned_storage_is_restricted_before_use() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let store = fixture.store();
    store.identity("agent").unwrap();
    let path = store.root().to_owned();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(
        path.join("identity.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    drop(store);
    let store = Storage::new(path.clone()).unwrap();
    store.load_identity("agent").unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(path.join("identity.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn inventory_overflow_is_explicit_and_new_records_are_bounded() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store.identity("agent").unwrap();
    for number in 0..(DISCOVERY_LIMIT - 2) {
        std::fs::write(store.root().join(format!("extra-{number}")), b"unused").unwrap();
    }
    assert!(store.identity("new").is_err());
    assert_eq!(store.identities().unwrap().len(), 1);
    std::fs::write(store.root().join("overflow"), b"unused").unwrap();
    assert!(store.identities().is_err());
    assert!(store.discover(&"a".repeat(64), true).is_err());
}

#[test]
fn read_only_open_neither_creates_nor_repairs_storage() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    assert!(Storage::open_existing(fixture.0.join("missing")).is_err());
    assert!(!fixture.0.join("missing").exists());
    let store = fixture.store();
    let identity = store.identity("agent").unwrap();
    let readonly = Storage::open_existing(store.root().to_owned()).unwrap();
    assert_eq!(
        readonly
            .load_identity("agent")
            .unwrap()
            .unwrap()
            .credential(),
        identity.credential()
    );
    assert!(
        readonly
            .grant(&fixture.project(), &identity, BTreeSet::new())
            .is_err()
    );
    std::fs::set_permissions(
        store.root().join("identity.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(readonly.load_identity("agent").is_err());
    assert_eq!(
        std::fs::metadata(store.root().join("identity.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    std::fs::set_permissions(store.root(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Storage::open_existing(store.root().to_owned()).is_err());
    assert_eq!(
        std::fs::metadata(store.root())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}
