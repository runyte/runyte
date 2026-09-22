// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use serde_json::value::RawValue;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::fs::OpenOptionsExt,
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    sync::Arc,
};
use windows_sys::Win32::{
    Storage::FileSystem::FILE_ALL_ACCESS, System::Threading::CREATE_NO_WINDOW,
};

const FIXTURE_ROOT: &str = "RUNYTE_PLUGIN_STATE_FIXTURE_ROOT";
const FIXTURE_MODE: &str = "RUNYTE_PLUGIN_STATE_FIXTURE_MODE";
const FIXTURE_NAME: &str = "plugin::state::windows_tests::native_state_process_fixture";

fn document(value: &str) -> Document {
    Document {
        version: 1,
        data: RawValue::from_string(value.into()).unwrap(),
    }
}

fn set(anchor: &Path, root: &Path, revision: &str, value: &str) -> Result<Info, Error> {
    run_with_anchor(
        root,
        Some(anchor),
        "tasks",
        Task::Set {
            expected_revision: revision.into(),
            document: document(value),
        },
        &Control::new(),
    )
}

fn get(anchor: &Path, root: &Path) -> Info {
    run_with_anchor(root, Some(anchor), "tasks", Task::Get, &Control::new()).unwrap()
}

#[test]
fn anchored_native_state_canonical_cas_delete_and_locking() {
    let anchor = TestRuntimeRoot::new("native-state-anchor").unwrap();
    let workspace = anchor.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = workspace.join("state");
    assert_eq!(get(anchor.path(), &store).revision, "s:missing");
    let first = set(
        anchor.path(),
        &store,
        "s:missing",
        "{\"z\":2,\"a\":\"cat\"}",
    )
    .unwrap();
    assert_eq!(
        first.document.as_ref().unwrap().get(),
        "{\"data\":{\"a\":\"cat\",\"z\":2},\"version\":1}"
    );
    assert_eq!(
        set(anchor.path(), &store, "s:missing", "null")
            .unwrap_err()
            .code,
        Code::Conflict
    );

    let directory =
        crate::private_storage::Directory::open_existing(&store.join("plugins/tasks"), true)
            .unwrap();
    let lock =
        storage::StateLock::acquire(directory.append(std::ffi::OsStr::new(".lock")).unwrap())
            .unwrap();
    assert_eq!(
        run_with_anchor(
            &store,
            Some(anchor.path()),
            "tasks",
            Task::Get,
            &Control::new(),
        )
        .unwrap_err()
        .code,
        Code::Busy
    );
    drop(lock);

    let deleted = run_with_anchor(
        &store,
        Some(anchor.path()),
        "tasks",
        Task::Delete {
            expected_revision: first.revision,
        },
        &Control::new(),
    )
    .unwrap();
    assert_eq!(deleted.revision, "s:missing");
    assert!(!store.join("plugins/tasks/state.pending").exists());
}

#[test]
fn anchored_native_state_recovers_pending_and_reports_post_promotion_uncertainty() {
    let anchor = TestRuntimeRoot::new("native-state-promotion").unwrap();
    let workspace = anchor.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = workspace.join("state");
    let first = set(anchor.path(), &store, "s:missing", "1").unwrap();
    let pending = store.join("plugins/tasks/state.pending");
    let directory =
        crate::private_storage::Directory::open_existing(&store.join("plugins/tasks"), true)
            .unwrap();
    let mut interrupted = directory
        .create_new(std::ffi::OsStr::new("state.pending"))
        .unwrap();
    interrupted.write_all(b"orphan").unwrap();
    interrupted.sync_all().unwrap();
    drop(interrupted);
    drop(directory);
    assert_eq!(get(anchor.path(), &store).revision, first.revision);
    assert!(!pending.exists());

    let cancelled = Arc::new(Control::new());
    let weak = Arc::downgrade(&cancelled);
    cancelled.set_hook(Arc::new(move |phase| {
        if phase == Checkpoint::BeforeMutation {
            weak.upgrade().unwrap().cancel();
        }
        Ok(())
    }));
    assert_eq!(
        run_with_anchor(
            &store,
            Some(anchor.path()),
            "tasks",
            Task::Set {
                expected_revision: first.revision.clone(),
                document: document("2"),
            },
            &cancelled,
        )
        .unwrap_err()
        .code,
        Code::Cancelled
    );
    assert_eq!(get(anchor.path(), &store).revision, first.revision);

    let uncertain = Control::new();
    uncertain.set_hook(Arc::new(|phase| {
        if phase == Checkpoint::AfterMutation {
            Err(unavailable())
        } else {
            Ok(())
        }
    }));
    assert_eq!(
        run_with_anchor(
            &store,
            Some(anchor.path()),
            "tasks",
            Task::Set {
                expected_revision: first.revision,
                document: document("3"),
            },
            &uncertain,
        )
        .unwrap_err()
        .code,
        Code::OutcomeUnknown
    );
    assert_eq!(
        get(anchor.path(), &store).document.unwrap().get(),
        "{\"data\":3,\"version\":1}"
    );
}

#[test]
fn anchored_native_state_refuses_missing_parents_and_paths_outside_anchor() {
    let anchor = TestRuntimeRoot::new("native-state-containment").unwrap();
    let outside = TestRuntimeRoot::new("native-state-outside").unwrap();
    let control = Control::new();
    assert_eq!(
        run_with_anchor(
            &anchor.join("missing/parents/state"),
            Some(anchor.path()),
            "tasks",
            Task::Get,
            &control,
        )
        .unwrap_err()
        .code,
        Code::Unavailable
    );
    assert!(!anchor.join("missing").exists());
    assert_eq!(
        run_with_anchor(
            &outside.join("state"),
            Some(anchor.path()),
            "tasks",
            Task::Get,
            &control,
        )
        .unwrap_err()
        .code,
        Code::Unavailable
    );
    assert!(!outside.join("state").exists());
}

#[test]
fn anchored_native_state_rejects_linked_and_nonprivate_documents() {
    let linked_anchor = TestRuntimeRoot::new("native-state-linked").unwrap();
    fs::create_dir(linked_anchor.join("workspace")).unwrap();
    let linked_store = linked_anchor.join("workspace/state");
    assert_eq!(
        get(linked_anchor.path(), &linked_store).revision,
        "s:missing"
    );
    let victim = linked_anchor.join("victim");
    fs::write(&victim, b"outside").unwrap();
    fs::hard_link(&victim, linked_store.join("plugins/tasks/state.json")).unwrap();
    assert_eq!(
        run_with_anchor(
            &linked_store,
            Some(linked_anchor.path()),
            "tasks",
            Task::Get,
            &Control::new(),
        )
        .unwrap_err()
        .code,
        Code::Unavailable
    );
    assert_eq!(fs::read(&victim).unwrap(), b"outside");

    let acl_anchor = TestRuntimeRoot::new("native-state-acl").unwrap();
    fs::create_dir(acl_anchor.join("workspace")).unwrap();
    let acl_store = acl_anchor.join("workspace/state");
    set(acl_anchor.path(), &acl_store, "s:missing", "1").unwrap();
    let file = OpenOptions::new()
        .access_mode(FILE_ALL_ACCESS)
        .open(acl_store.join("plugins/tasks/state.json"))
        .unwrap();
    crate::private_storage::test_set_acl(&file, "D:P(A;;GA;;;OW)(A;;GR;;;WD)").unwrap();
    assert_eq!(
        run_with_anchor(
            &acl_store,
            Some(acl_anchor.path()),
            "tasks",
            Task::Get,
            &Control::new(),
        )
        .unwrap_err()
        .code,
        Code::Unavailable
    );
    assert_eq!(
        fs::read(acl_store.join("plugins/tasks/state.json")).unwrap(),
        b"{\"data\":1,\"version\":1}"
    );
}

#[test]
#[ignore = "compiled helper for the native cross-process plugin-state test"]
fn native_state_process_fixture() {
    let Some(anchor) = std::env::var_os(FIXTURE_ROOT).map(std::path::PathBuf::from) else {
        return;
    };
    let store = anchor.join("workspace/state");
    match std::env::var(FIXTURE_MODE).unwrap().as_str() {
        "busy" => assert_eq!(
            run_with_anchor(&store, Some(&anchor), "tasks", Task::Get, &Control::new(),)
                .unwrap_err()
                .code,
            Code::Busy
        ),
        "set" => {
            set(&anchor, &store, "s:missing", "7").unwrap();
        }
        mode => panic!("unknown fixture mode {mode}"),
    }
}

fn fixture(anchor: &Path, mode: &str) -> std::process::ExitStatus {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", FIXTURE_NAME, "--ignored", "--nocapture"])
        .env(FIXTURE_ROOT, anchor)
        .env(FIXTURE_MODE, mode)
        .env("XDG_CONFIG_HOME", anchor.join("config"))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
}

#[test]
fn independent_process_observes_lock_and_commits_after_release() {
    let anchor = TestRuntimeRoot::new("native-state-process").unwrap();
    fs::create_dir(anchor.join("workspace")).unwrap();
    let store = anchor.join("workspace/state");
    assert_eq!(get(anchor.path(), &store).revision, "s:missing");
    let directory =
        crate::private_storage::Directory::open_existing(&store.join("plugins/tasks"), true)
            .unwrap();
    let lock =
        storage::StateLock::acquire(directory.append(std::ffi::OsStr::new(".lock")).unwrap())
            .unwrap();
    assert!(fixture(anchor.path(), "busy").success());
    drop(lock);
    assert!(fixture(anchor.path(), "set").success());
    assert_eq!(
        get(anchor.path(), &store).document.unwrap().get(),
        "{\"data\":7,\"version\":1}"
    );
}
