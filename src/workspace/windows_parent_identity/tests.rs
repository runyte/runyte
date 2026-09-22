// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{private_storage::Directory, test_support::TestRuntimeRoot};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    io::{self, Read, Write},
    mem::size_of,
    os::windows::{
        io::{AsHandle, AsRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{
    JobObjects::{
        AssignProcessToJobObject, IsProcessInJob, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        JobObjectBasicAccountingInformation, QueryInformationJobObject, TerminateJobObject,
    },
    Threading::{CREATE_NO_WINDOW, WaitForSingleObject},
};

const HELPER_NAME: &str = "workspace::windows_parent_identity::tests::process_helper";
const MODE_ENV: &str = "RUNYTE_PARENT_ID_MODE";
const ROOT_ENV: &str = "RUNYTE_PARENT_ID_ROOT";
const READY_BUDGET: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize, Serialize)]
struct Report {
    child_pid: u32,
    parent: ProcessIdentity,
}

struct OwnedChild(Child);

impl OwnedChild {
    fn spawn(root: &Path, mode: &str, stdin: Stdio) -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", HELPER_NAME, "--ignored", "--nocapture"])
                .env(MODE_ENV, mode)
                .env(ROOT_ENV, root)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(stdin)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    fn wait(&mut self) {
        let deadline = Instant::now() + READY_BUDGET;
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "parent fixture failed: {status}");
                return;
            }
            assert!(Instant::now() < deadline, "parent fixture did not exit");
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            unsafe {
                WaitForSingleObject(self.0.as_raw_handle(), 5000);
            }
            let _ = self.0.try_wait();
        }
    }
}

/// Owns every process in the nested fixture through completion or unwind.
/// Storage stays alive if Windows cannot prove the job has drained.
struct JobTree {
    job: OwnedHandle,
    storage: Option<TestRuntimeRoot>,
    joined: bool,
}

impl JobTree {
    fn new() -> Self {
        let storage = TestRuntimeRoot::new("parent-identity-exit").unwrap();
        let job = crate::windows_process::new_job().unwrap();
        Self {
            job,
            storage: Some(storage),
            joined: false,
        }
    }

    fn root(&self) -> &TestRuntimeRoot {
        self.storage.as_ref().unwrap()
    }

    fn shutdown(&mut self) -> io::Result<()> {
        if self.joined {
            return Ok(());
        }
        if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let deadline = Instant::now() + READY_BUDGET;
        loop {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            if unsafe {
                QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if accounting.ActiveProcesses == 0 {
                self.joined = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "fixture job still has live processes",
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for JobTree {
    fn drop(&mut self) {
        if self.shutdown().is_err() {
            // Do not delete a fixture root while an unjoined descendant may
            // still be using it. This is an exceptional test failure path.
            if let Some(storage) = self.storage.take() {
                std::mem::forget(storage);
            }
        }
    }
}

fn wait_for_file(child: &mut OwnedChild, path: &Path) {
    let deadline = Instant::now() + READY_BUDGET;
    while !path.exists() {
        if let Some(status) = child.0.try_wait().unwrap() {
            if path.exists() {
                assert!(status.success(), "fixture exited unsuccessfully: {status}");
                break;
            }
            panic!("fixture exited before {}: {status}", path.display());
        }
        assert!(
            Instant::now() < deadline,
            "fixture did not write {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn root() -> PathBuf {
    PathBuf::from(std::env::var_os(ROOT_ENV).unwrap())
}

fn publish_report(root: &Path, name: &str, report: &Report) {
    Directory::open_existing(root, true)
        .unwrap()
        .atomic_write(OsStr::new(name), &serde_json::to_vec(report).unwrap())
        .unwrap();
}

fn retained_parent() -> RetainedParent {
    match observe_parent(ParentRole::Foreground).unwrap() {
        ParentObservation::Retained(parent) => parent,
        ParentObservation::Unavailable(reason) => panic!("fixture parent unavailable: {reason:?}"),
    }
}

#[test]
#[ignore]
fn process_helper() {
    let Some(mode) = std::env::var_os(MODE_ENV) else {
        return;
    };
    let root = root();
    match mode.to_str().unwrap() {
        "direct" => {
            let parent = retained_parent();
            assert!(parent.parent().is_alive().unwrap());
            let report = Report {
                child_pid: parent.child().pid,
                parent: parent.parent().identity(),
            };
            publish_report(&root, "direct-report", &report);
            let mut sink = Vec::new();
            std::io::stdin().read_to_end(&mut sink).unwrap();
        }
        "parent" => {
            std::fs::write(root.join("parent-ready"), b"ready").unwrap();
            let mut go = [0u8; 1];
            std::io::stdin().read_exact(&mut go).unwrap();
            let mut grandchild = OwnedChild::spawn(&root, "grandchild", Stdio::null());
            wait_for_file(&mut grandchild, &root.join("grandchild-captured"));
            // The enclosing test owns a kill-on-close job for this tree. Exit
            // normally now so the grandchild can observe its retained handle.
            std::mem::forget(grandchild);
        }
        "grandchild" => {
            let parent = retained_parent();
            publish_report(
                &root,
                "grandchild-captured",
                &Report {
                    child_pid: parent.child().pid,
                    parent: parent.parent().identity(),
                },
            );
            let deadline = Instant::now() + READY_BUDGET;
            while parent.parent().is_alive().unwrap() {
                assert!(Instant::now() < deadline, "retained parent did not exit");
                thread::sleep(Duration::from_millis(5));
            }
            std::fs::write(root.join("grandchild-saw-exit"), b"exited").unwrap();
            let deadline = Instant::now() + READY_BUDGET;
            while !root.join("release-grandchild").exists() {
                assert!(Instant::now() < deadline, "grandchild was not released");
                thread::sleep(Duration::from_millis(5));
            }
        }
        _ => panic!("unknown parent fixture mode"),
    }
}

#[test]
fn foreground_child_pins_its_actual_live_parent() {
    let storage = TestRuntimeRoot::new("parent-identity-direct").unwrap();
    let mut child = OwnedChild::spawn(storage.path(), "direct", Stdio::piped());
    wait_for_file(&mut child, &storage.join("direct-report"));
    let report: Report =
        serde_json::from_slice(&std::fs::read(storage.join("direct-report")).unwrap()).unwrap();
    assert_eq!(report.child_pid, child.0.id());
    assert_eq!(report.parent, ProcessIdentity::current().unwrap());
    assert!(child.0.try_wait().unwrap().is_none());
    drop(child.0.stdin.take());
    child.wait();
}

#[test]
fn parent_exit_after_pin_stays_bound_to_the_original_process() {
    let mut tree = JobTree::new();
    let mut parent = OwnedChild::spawn(tree.root().path(), "parent", Stdio::piped());
    wait_for_file(&mut parent, &tree.root().join("parent-ready"));
    let original = PinnedProcess::open_peer(parent.0.id()).unwrap();
    assert_ne!(
        unsafe { AssignProcessToJobObject(tree.job.as_raw_handle(), parent.0.as_raw_handle()) },
        0,
        "cannot own parent fixture's entire child tree: {}",
        io::Error::last_os_error()
    );
    parent.0.stdin.as_mut().unwrap().write_all(b"g").unwrap();
    wait_for_file(&mut parent, &tree.root().join("grandchild-captured"));
    let report: Report =
        serde_json::from_slice(&std::fs::read(tree.root().join("grandchild-captured")).unwrap())
            .unwrap();
    assert_eq!(report.parent, original.identity());
    let grandchild = PinnedProcess::open_peer(report.child_pid).unwrap();
    let mut contained = 0;
    assert_ne!(
        unsafe {
            IsProcessInJob(
                grandchild.as_handle().as_raw_handle(),
                tree.job.as_raw_handle(),
                &mut contained,
            )
        },
        0
    );
    assert_ne!(
        contained, 0,
        "grandchild escaped the fixture-owned cleanup job"
    );
    parent.wait();
    let deadline = Instant::now() + READY_BUDGET;
    while !tree.root().join("grandchild-saw-exit").exists() {
        assert!(
            Instant::now() < deadline,
            "grandchild did not observe original parent's exit"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!original.is_alive().unwrap());
    assert!(grandchild.is_alive().unwrap());
    std::fs::write(tree.root().join("release-grandchild"), b"go").unwrap();
    let deadline = Instant::now() + READY_BUDGET;
    while grandchild.is_alive().unwrap() {
        assert!(Instant::now() < deadline, "grandchild did not exit");
        thread::sleep(Duration::from_millis(5));
    }
    tree.shutdown().unwrap();
}

#[test]
fn later_created_candidate_and_self_pid_are_refused() {
    let child_identity = ProcessIdentity::current().unwrap();
    let storage = TestRuntimeRoot::new("parent-identity-order").unwrap();
    let mut later = OwnedChild::spawn(storage.path(), "parent", Stdio::piped());
    wait_for_file(&mut later, &storage.join("parent-ready"));
    let error = pin_candidate(child_identity, later.0.id()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("created after"));
    assert_eq!(
        pin_candidate(child_identity, child_identity.pid)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(matches!(
        pin_candidate(child_identity, 0).unwrap(),
        ParentObservation::Unavailable(ParentUnavailable::NoParentId)
    ));
}

#[test]
fn detached_host_never_observes_its_inheritance_surrogate() {
    assert!(matches!(
        observe_parent(ParentRole::DetachedHost).unwrap(),
        ParentObservation::Unavailable(ParentUnavailable::DetachedHost)
    ));
}
