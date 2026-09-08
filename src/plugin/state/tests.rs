// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
fn document(value: &str) -> Document {
    Document {
        version: 1,
        data: RawValue::from_string(value.into()).unwrap(),
    }
}
fn set(root: &Path, expected: &str, value: &str) -> Result<Info, Error> {
    run(
        root,
        "tasks",
        Task::Set {
            expected_revision: expected.into(),
            document: document(value),
        },
        &Control::new(),
    )
}
fn get(root: &Path) -> Info {
    run(root, "tasks", Task::Get, &Control::new()).unwrap()
}

#[test]
fn private_state_canonical_cas_delete_and_content_aba() {
    let root = TestRuntimeRoot::new("state-cas").unwrap();
    let store = root.join("state");
    assert_eq!(get(&store).revision, "s:missing");
    let first = set(&store, "s:missing", "{\"z\":2,\"a\":\"猫\"}").unwrap();
    assert_eq!(
        first.document.as_ref().unwrap().get(),
        "{\"data\":{\"a\":\"猫\",\"z\":2},\"version\":1}"
    );
    assert_eq!(
        set(&store, "s:missing", "null").unwrap_err().code,
        Code::Conflict
    );
    let second = set(&store, &first.revision, "[]").unwrap();
    let third = set(&store, &second.revision, "{\"a\":\"猫\",\"z\":2}").unwrap();
    assert_eq!(first.revision, third.revision);
    let directory = store.join("plugins/tasks");
    for path in [&store, &store.join("plugins"), &directory] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for name in ["state.json", ".lock"] {
        assert_eq!(
            fs::metadata(directory.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let deleted = run(
        &store,
        "tasks",
        Task::Delete {
            expected_revision: first.revision,
        },
        &Control::new(),
    )
    .unwrap();
    assert_eq!(deleted.revision, "s:missing");
    assert!(deleted.document.is_none());
    assert!(!directory.join("state.pending").exists());
}

#[test]
fn private_state_refuses_symlink_hardlink_fifo_and_nonprivate_targets() {
    for name in ["state.json", "state.pending", ".lock"] {
        for kind in ["symlink", "hardlink", "fifo"] {
            let root = TestRuntimeRoot::new("state-links").unwrap();
            let store = root.join("state");
            get(&store);
            let victim = root.join("victim");
            fs::write(&victim, b"outside").unwrap();
            let path = store.join("plugins/tasks").join(name);
            let _ = fs::remove_file(&path);
            match kind {
                "symlink" => symlink(&victim, &path).unwrap(),
                "hardlink" => fs::hard_link(&victim, &path).unwrap(),
                _ => {
                    use std::os::unix::ffi::OsStrExt;
                    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                }
            }
            assert!(
                run(&store, "tasks", Task::Get, &Control::new()).is_err(),
                "{name}/{kind}"
            );
            assert_eq!(fs::read(&victim).unwrap(), b"outside");
        }
    }
    let root = TestRuntimeRoot::new("state-modes").unwrap();
    let store = root.join("state");
    let saved = set(&store, "s:missing", "null").unwrap();
    fs::set_permissions(
        store.join("plugins/tasks/state.json"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(
        run(
            &store,
            "tasks",
            Task::Delete {
                expected_revision: saved.revision
            },
            &Control::new()
        )
        .is_err()
    );
}

#[test]
fn private_state_refuses_linked_directories_and_serializes_independent_callers() {
    use std::os::fd::AsRawFd;
    let root = TestRuntimeRoot::new("state-lock").unwrap();
    let store = root.join("state");
    get(&store);
    let directory =
        crate::private_storage::Directory::open(&store.join("plugins/tasks"), true).unwrap();
    let lock = directory.append(OsStr::new(".lock")).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert_eq!(
        run(&store, "tasks", Task::Get, &Control::new())
            .unwrap_err()
            .code,
        Code::Busy
    );
    drop(lock);
    assert_eq!(get(&store).revision, "s:missing");
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::remove_dir_all(store.join("plugins/tasks")).unwrap();
    symlink(&outside, store.join("plugins/tasks")).unwrap();
    assert!(run(&store, "tasks", Task::Get, &Control::new()).is_err());
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

#[test]
fn private_state_fixed_pending_recovery_cancel_and_postpromotion_failure() {
    let root = TestRuntimeRoot::new("state-atomic").unwrap();
    let store = root.join("state");
    let first = set(&store, "s:missing", "1").unwrap();
    let directory = store.join("plugins/tasks");
    fs::write(directory.join("state.pending"), b"interrupted write").unwrap();
    assert_eq!(get(&store).revision, first.revision);
    assert!(!directory.join("state.pending").exists());
    let control = Arc::new(Control::new());
    let cancelled = Arc::downgrade(&control);
    control.set_hook(Arc::new(move |phase| {
        if phase == Checkpoint::BeforeMutation {
            cancelled.upgrade().unwrap().cancel();
        }
        Ok(())
    }));
    assert_eq!(
        run(
            &store,
            "tasks",
            Task::Set {
                expected_revision: first.revision.clone(),
                document: document("2")
            },
            &control
        )
        .unwrap_err()
        .code,
        Code::Cancelled
    );
    assert_eq!(get(&store).revision, first.revision);
    assert!(!directory.join("state.pending").exists());
    let control = Control::new();
    control.set_hook(Arc::new(|phase| {
        if phase == Checkpoint::AfterMutation {
            Err(unavailable())
        } else {
            Ok(())
        }
    }));
    assert_eq!(
        run(
            &store,
            "tasks",
            Task::Set {
                expected_revision: first.revision,
                document: document("3")
            },
            &control
        )
        .unwrap_err()
        .code,
        Code::OutcomeUnknown
    );
    assert_eq!(
        get(&store).document.unwrap().get(),
        "{\"data\":3,\"version\":1}"
    );
    assert!(!directory.join("state.pending").exists());
}

#[test]
fn private_state_bounds_and_corrupt_document_do_not_overwrite_existing_content() {
    let root = TestRuntimeRoot::new("state-bounds").unwrap();
    let store = root.join("state");
    let first = set(&store, "s:missing", "null").unwrap();
    let too_many = format!("[{}]", vec!["null"; 1025].join(","));
    assert_eq!(
        set(&store, &first.revision, &too_many).unwrap_err().code,
        Code::LimitExceeded
    );
    assert_eq!(get(&store).revision, first.revision);
    let path = store.join("plugins/tasks/state.json");
    fs::write(&path, b"not JSON").unwrap();
    assert_eq!(
        run(&store, "tasks", Task::Get, &Control::new())
            .unwrap_err()
            .code,
        Code::InvalidArgument
    );
    assert!(set(&store, &first.revision, "1").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"not JSON");
}

#[test]
fn state_wire_raw_preflight_is_order_independent_and_does_not_bound_view_rows() {
    let request=b"{\"params\":{\"document\":{\"data\":null,\"version\":1},\"expected_revision\":\"s:missing\"},\"method\":\"state.set\",\"id\":\"p:1\",\"type\":\"request\"}";
    let super::super::ClientMessage::Application(
        super::super::application::ClientMessage::Request {
            request: super::super::application::Request::StateSet { document, .. },
            ..
        },
    ) = super::super::application::decode(request).unwrap()
    else {
        panic!()
    };
    assert_eq!(document.data.get(), "null");
    let large = format!(
        "{{\"type\":\"request\",\"id\":\"p:2\",\"method\":\"state.set\",\"params\":{{\"expected_revision\":\"s:missing\",\"document\":{{\"version\":1,\"data\":[{}]}}}}}}",
        vec!["0"; 1025].join(",")
    );
    assert!(super::super::application::decode(large.as_bytes()).is_err());
    let unrelated = format!(
        "{{\"type\":\"request\",\"id\":\"p:3\",\"method\":\"view.create\",\"params\":{{\"unrelated\":[{}]}}}}",
        vec!["0"; 10000].join(",")
    );
    assert!(wire::preflight(unrelated.as_bytes()).unwrap().0.is_none());
    assert!(super::super::application::decode(br#"{"type":"request","id":"p:4","method":"state.set","params":{"expected_revision":"s:missing","document":null}}"#).is_err());
}

#[test]
fn state_integer_ids_are_exact_or_rejected_in_wire_and_existing_storage() {
    let root = TestRuntimeRoot::new("state-integers").unwrap();
    let store = root.join("state");
    let valid = "[-9223372036854775808,18446744073709551615,1e30,1.5,\"18446744073709551616\"]";
    let saved = set(&store, "s:missing", valid).unwrap();
    assert!(
        saved
            .document
            .as_ref()
            .unwrap()
            .get()
            .contains("18446744073709551615")
    );
    for number in ["18446744073709551616", "-9223372036854775809"] {
        assert_eq!(
            set(&store, &saved.revision, number).unwrap_err().code,
            Code::InvalidArgument
        );
        let wire = format!(
            "{{\"type\":\"request\",\"id\":\"p:1\",\"method\":\"state.set\",\"params\":{{\"expected_revision\":\"s:missing\",\"document\":{{\"version\":1,\"data\":{number}}}}}}}"
        );
        assert!(super::super::application::decode(wire.as_bytes()).is_err());
        fs::write(
            store.join("plugins/tasks/state.json"),
            format!("{{\"version\":1,\"data\":{number}}}"),
        )
        .unwrap();
        assert_eq!(
            run(&store, "tasks", Task::Get, &Control::new())
                .unwrap_err()
                .code,
            Code::InvalidArgument
        );
    }
}
