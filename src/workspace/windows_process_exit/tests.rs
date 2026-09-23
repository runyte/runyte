// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{
    future::Future,
    io::Read,
    os::windows::io::AsRawHandle,
    os::windows::process::CommandExt,
    pin::Pin,
    process::{Child, Command, Stdio},
    sync::mpsc,
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
};
use tokio::runtime::{Builder, Runtime};
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, WaitForSingleObject};

const CHILD_NAME: &str = "workspace::windows_process_exit::tests::simple_child";
const OVERLAP_NAME: &str =
    "workspace::windows_process_exit::tests::callback_and_drop_overlap_helper";
const REENTRANT_NAME: &str = "workspace::windows_process_exit::tests::reentrant_helper";
const CHILD_ENV: &str = "RUNYTE_EXIT_WATCHER_CHILD";
const CHILD_READY: &str = "RUNYTE_EXIT_WATCHER_READY";
const OVERLAP_ENV: &str = "RUNYTE_EXIT_WATCHER_OVERLAP";
const REENTRANT_ENV: &str = "RUNYTE_EXIT_WATCHER_REENTRANT";
const COMPLETED_ENV: &str = "RUNYTE_EXIT_WATCHER_COMPLETED";

pub(super) struct CallbackHook {
    pub(super) entered: mpsc::Sender<()>,
    pub(super) release: Mutex<mpsc::Receiver<()>>,
}

fn runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

struct ChildFixture {
    child: Child,
    _root: TestRuntimeRoot,
}

impl ChildFixture {
    fn spawn() -> Self {
        let root = TestRuntimeRoot::new("process-exit-watcher").unwrap();
        let ready_path = root.join("ready");
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_NAME, "--ignored", "--nocapture"])
            .env(CHILD_ENV, "1")
            .env(CHILD_READY, &ready_path)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut fixture = Self { child, _root: root };
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready_path.exists() {
            assert!(
                fixture.child.try_wait().unwrap().is_none(),
                "child exited before readiness"
            );
            assert!(Instant::now() < deadline, "child did not publish readiness");
            thread::sleep(Duration::from_millis(5));
        }
        fixture
    }

    fn pin(&self) -> Arc<PinnedProcess> {
        Arc::new(PinnedProcess::open_peer(self.child.id()).unwrap())
    }

    fn release(&mut self) {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "child exited unsuccessfully: {status}");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "child did not exit after stdin closed"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for ChildFixture {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            unsafe {
                WaitForSingleObject(self.child.as_raw_handle(), 5000);
            }
            let _ = self.child.try_wait();
        }
    }
}

#[test]
#[ignore]
fn simple_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    std::fs::write(std::env::var_os(CHILD_READY).unwrap(), b"ready").unwrap();
    let mut sink = Vec::new();
    std::io::stdin().read_to_end(&mut sink).unwrap();
}

#[test]
fn already_exited_process_notifies_and_repeated_waits_stay_ready() {
    let mut child = ChildFixture::spawn();
    let process = child.pin();
    child.release();
    let runtime = runtime();
    runtime.block_on(async {
        let watcher = ProcessExitWatcher::new(process).unwrap();
        tokio::time::timeout(Duration::from_secs(3), watcher.wait())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_millis(100), watcher.wait())
            .await
            .unwrap();
    });
}

#[test]
fn cancelled_wait_keeps_sticky_exit_for_next_waiter() {
    let mut child = ChildFixture::spawn();
    let process = child.pin();
    let runtime = runtime();
    runtime.block_on(async {
        let watcher = ProcessExitWatcher::new(process).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), watcher.wait())
                .await
                .is_err()
        );
        child.release();
        tokio::time::timeout(Duration::from_secs(3), watcher.wait())
            .await
            .unwrap();
        watcher.wait().await;
    });
}

#[test]
#[ignore]
fn callback_and_drop_overlap_helper() {
    if std::env::var_os(OVERLAP_ENV).is_none() {
        return;
    }
    let mut child = ChildFixture::spawn();
    let process = child.pin();
    let runtime = runtime();
    let watcher = {
        let _enter = runtime.enter();
        ProcessExitWatcher::new(process).unwrap()
    };
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let hook = Arc::new(CallbackHook {
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    unsafe {
        (*watcher.bundle)
            .callback_hook
            .lock()
            .unwrap()
            .replace(Arc::clone(&hook));
    }
    child.release();
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let drop_thread = thread::spawn(move || {
        started_tx.send(()).unwrap();
        drop(watcher);
        done_tx.send(()).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let completed_early = done_rx.recv_timeout(Duration::from_millis(40));
    release_tx.send(()).unwrap();
    assert!(completed_early.is_err());
    done_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    drop_thread.join().unwrap();
    std::fs::write(std::env::var_os(COMPLETED_ENV).unwrap(), b"completed").unwrap();
}

#[test]
fn callback_and_drop_overlap_keeps_bundle_until_completed_unregister() {
    run_bounded_helper(OVERLAP_NAME, OVERLAP_ENV);
}

#[test]
fn runtime_can_shut_down_before_callback_and_unregister() {
    let mut child = ChildFixture::spawn();
    let process = child.pin();
    let runtime = runtime();
    let watcher = {
        let _enter = runtime.enter();
        ProcessExitWatcher::new(process).unwrap()
    };
    drop(runtime);
    child.release();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !watcher.has_exited() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(watcher.has_exited());
    drop(watcher);
}

#[test]
fn registration_failure_and_missing_runtime_release_the_process() {
    let child = ChildFixture::spawn();
    let process = child.pin();
    let error = match ProcessExitWatcher::new(Arc::clone(&process)) {
        Ok(_) => panic!("watcher unexpectedly registered without a Tokio runtime"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("no Tokio runtime"));
    let runtime = runtime();
    {
        let _enter = runtime.enter();
        let error = match ProcessExitWatcher::new_with_registration(Arc::clone(&process), |_, _| {
            Err(io::Error::other("injected registration failure"))
        }) {
            Ok(_) => panic!("injected registration failure unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "injected registration failure");
    }
    assert_eq!(Arc::strong_count(&process), 1);
}

#[test]
fn failed_unregister_retains_process_and_callback_context() {
    let child = ChildFixture::spawn();
    let process = child.pin();
    let runtime = runtime();
    let mut watcher = {
        let _enter = runtime.enter();
        ProcessExitWatcher::new(Arc::clone(&process)).unwrap()
    };
    let actual_registration = watcher.registration;
    let bundle = watcher.bundle;
    watcher.registration = ptr::null_mut(); // Inject an invalid unregister token.
    drop(watcher);
    assert_eq!(Arc::strong_count(&process), 2);
    assert_ne!(
        unsafe { UnregisterWaitEx(actual_registration, INVALID_HANDLE_VALUE) },
        0
    );
    unsafe {
        drop(Box::from_raw(bundle));
    }
    assert_eq!(Arc::strong_count(&process), 1);
}

struct DropOnWake {
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,
    done: AtomicBool,
}

impl Wake for DropOnWake {
    fn wake(self: Arc<Self>) {
        let future = self.future.lock().unwrap().take();
        drop(future);
        self.done.store(true, Ordering::Release);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let future = self.future.lock().unwrap().take();
        drop(future);
        self.done.store(true, Ordering::Release);
    }
}

#[test]
#[ignore]
fn reentrant_helper() {
    if std::env::var_os(REENTRANT_ENV).is_none() {
        return;
    }
    let mut child = ChildFixture::spawn();
    let process = child.pin();
    let runtime = runtime();
    runtime.block_on(async {
        let watcher = ProcessExitWatcher::new(process).unwrap();
        let mut future = Box::pin(async move { watcher.wait().await });
        let dropper = Arc::new(DropOnWake {
            future: Mutex::new(None),
            done: AtomicBool::new(false),
        });
        let waker = Waker::from(Arc::clone(&dropper));
        let mut context = Context::from_waker(&waker);
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
        *dropper.future.lock().unwrap() = Some(future);
        child.release();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !dropper.done.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    std::fs::write(std::env::var_os(COMPLETED_ENV).unwrap(), b"completed").unwrap();
}

#[test]
fn caller_waker_can_destroy_watcher_without_callback_deadlock() {
    run_bounded_helper(REENTRANT_NAME, REENTRANT_ENV);
}

fn run_bounded_helper(name: &str, marker: &str) {
    let root = TestRuntimeRoot::new("process-exit-reentrant-parent").unwrap();
    let completed = root.join("completed");
    let mut helper = ChildFixture {
        child: Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--ignored", "--nocapture"])
            .env(marker, "1")
            .env(COMPLETED_ENV, &completed)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        _root: root,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = helper.child.try_wait().unwrap() {
            assert!(status.success(), "process-exit helper failed: {status}");
            assert!(
                completed.exists(),
                "helper did not complete its watcher scenario: {name}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "process-exit helper hung: {name}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
