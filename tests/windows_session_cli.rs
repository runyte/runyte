// SPDX-License-Identifier: MPL-2.0

#![cfg(windows)]

use runyte::{
    app::App,
    command::parse_named_command,
    config::Config,
    git::{DeletionAuthorization, GitCliProvider, GitProvider, WorktreeCreate},
    input::{KeyCode, KeyStroke, Modifiers},
    protocol::{ClientRequest, HostResponse, TransportChange, encode_path},
    test_support::TestRuntimeRoot,
    workspace::{
        WorkspaceEvent,
        windows_catalog::remember,
        windows_endpoint::NameStore,
        windows_lifecycle::connect_control,
        windows_location::{CapturedRoots, LocationInputs, ResolvedLayout},
        windows_process_identity::{PinResult, PinnedProcess},
        windows_service::WorkspaceServiceOwner,
        windows_transport::LocalClient,
        workspace_id,
    },
};
use std::{
    ffi::OsStr,
    fs,
    mem::size_of,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{
    JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    },
    Threading::{CREATE_NO_WINDOW, GetCurrentProcess},
};

const BIN: &str = env!("CARGO_BIN_EXE_runyte");
const RESTART_ROOT_ENV: &str = "RUNYTE_RESTART_ACCEPTANCE_ROOT";
const RESTART_SUCCESS_FIXTURE: &str = "restart_success_fixture";
const RESTART_DENIAL_FIXTURE: &str = "restart_denial_fixture";

struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct CliChild(Option<Child>);

impl Drop for CliChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A restarted host is detached from the fixture's child handle. Keep an
/// independent stop owner until assertions and fixture storage are finished.
struct RestartCleanup<'a> {
    root: &'a TestRuntimeRoot,
    cwd: &'a Path,
    selector: &'a str,
    armed: bool,
}

impl Drop for RestartCleanup<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut command = Command::new(BIN);
        command
            .creation_flags(CREATE_NO_WINDOW)
            .args(["--session-stop", "--force", self.selector])
            .current_dir(self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        private_environment(&mut command, self.root);
        let Ok(mut child) = command.spawn() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(12);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn running_in_job() -> bool {
    let mut member = 0;
    assert_ne!(
        unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut member) },
        0
    );
    member != 0
}

fn restart_fixture_outer_root() -> PathBuf {
    PathBuf::from(std::env::var_os(RESTART_ROOT_ENV).expect("restart fixture root was passed"))
}

fn await_restart_fixture_admission(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.join("fixture-admitted").exists() {
        assert!(
            Instant::now() < deadline,
            "restart fixture was not admitted"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn restart_fixture_job(limit: u32) -> OwnedHandle {
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    assert!(!raw.is_null(), "{}", std::io::Error::last_os_error());
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = limit;
    assert_ne!(
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    job
}

fn run_restart_fixture(fixture: &str, budget: Duration) {
    let root = TestRuntimeRoot::new("native-cli-restart-fixture").unwrap();
    let job = restart_fixture_job(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE);
    let log = fs::File::create(root.join("restart-fixture.log")).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", fixture, "--ignored", "--nocapture"])
        .env(RESTART_ROOT_ENV, root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    private_environment(&mut command, &root);
    let mut child = CliChild(Some(command.spawn().unwrap()));
    assert_ne!(
        unsafe {
            AssignProcessToJobObject(
                job.as_raw_handle(),
                child.0.as_ref().unwrap().as_raw_handle(),
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    fs::write(root.join("fixture-admitted"), b"ready").unwrap();
    let deadline = Instant::now() + budget;
    while child.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "restart fixture timed out: {fixture}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let status = child.0.take().unwrap().wait().unwrap();
    assert!(
        status.success(),
        "restart fixture {fixture} failed: {}",
        fs::read_to_string(root.join("restart-fixture.log")).unwrap_or_default()
    );
}

fn enter_restart_breakaway_job() -> OwnedHandle {
    let job = restart_fixture_job(JOB_OBJECT_LIMIT_BREAKAWAY_OK);
    assert_ne!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    job
}

fn private_environment(command: &mut Command, root: &TestRuntimeRoot) {
    command
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("RUNYTE_INTERNAL_WINDOWS_LAYOUT");
}

fn layout(root: &TestRuntimeRoot, name: &str) -> ResolvedLayout {
    let project = root.join(name);
    fs::create_dir_all(&project).unwrap();
    ResolvedLayout::resolve(LocationInputs {
        project_root: project.clone(),
        state_root: project.join(".runyte"),
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            cache_home: Some(root.join("cache")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    })
    .unwrap()
}

fn start_host(root: &TestRuntimeRoot, layout: &ResolvedLayout) -> Host {
    start_host_in(root, layout, &root.join("cache"), None)
}

fn start_host_in(
    root: &TestRuntimeRoot,
    layout: &ResolvedLayout,
    cache: &Path,
    config: Option<&Path>,
) -> Host {
    let diagnostic_path = root.join(format!(
        "host-{}-{}-{}.log",
        layout.project_root().file_name().unwrap().to_string_lossy(),
        layout.state_root().file_name().unwrap().to_string_lossy(),
        cache.file_name().unwrap().to_string_lossy()
    ));
    let diagnostics = fs::File::create(&diagnostic_path).unwrap();
    let mut command = Command::new(BIN);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args(["--serve", "--detached-host", "--project-root"])
        .arg(layout.project_root())
        .current_dir(layout.project_root())
        .stdin(Stdio::null())
        .stdout(Stdio::from(diagnostics.try_clone().unwrap()))
        .stderr(Stdio::from(diagnostics));
    private_environment(&mut command, root);
    command.env("XDG_CACHE_HOME", cache);
    if let Some(config) = config {
        command.arg("--config").arg(config);
    }
    let mut host = Host(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let location = layout.publication_location().unwrap();
    loop {
        if location.read_ready().unwrap().is_some() {
            return host;
        }
        if let Some(status) = host.0.try_wait().unwrap() {
            panic!(
                "native host exited before publishing readiness: {status}; diagnostic: {}",
                fs::read_to_string(&diagnostic_path).unwrap_or_default()
            );
        }
        assert!(
            Instant::now() < deadline,
            "native host did not become ready"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn cli(root: &TestRuntimeRoot, cwd: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(BIN);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    private_environment(&mut command, root);
    let mut child = CliChild(Some(command.spawn().unwrap()));
    let deadline = Instant::now()
        + if args.contains(&"--session-restart") {
            Duration::from_secs(25)
        } else {
            Duration::from_secs(12)
        };
    while child.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            panic!("session CLI did not finish: {args:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.0.take().unwrap().wait_with_output().unwrap()
}

fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn make_unsaved(layout: &ResolvedLayout, file: &Path) -> (tokio::runtime::Runtime, LocalClient) {
    let metadata = layout
        .publication_location()
        .unwrap()
        .read_ready()
        .unwrap()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut control = runtime.block_on(connect_control(&metadata)).unwrap();
    runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                control
                    .send(&ClientRequest::OpenBuffers {
                        paths: vec![encode_path(file)],
                        activate: false,
                    })
                    .await
                    .unwrap();
                let Some(HostResponse::Opened { buffers }) = control.recv().await.unwrap() else {
                    panic!("host did not open fixture buffer");
                };
                let buffer = buffers[0];
                control
                    .send(&ClientRequest::ReadBuffer { buffer })
                    .await
                    .unwrap();
                let Some(HostResponse::Buffer { buffer: contents }) = control.recv().await.unwrap()
                else {
                    panic!("host did not read fixture buffer");
                };
                control
                    .send(&ClientRequest::ApplyTransaction {
                        buffer,
                        expected: contents.metadata.revision,
                        changes: vec![TransportChange {
                            from: 0,
                            to: 0,
                            text: "unsaved ".into(),
                        }],
                    })
                    .await
                    .unwrap();
                assert!(matches!(
                    control.recv().await.unwrap(),
                    Some(HostResponse::TransactionApplied { .. })
                ));
            })
            .await
        })
        .expect("native host did not answer bounded buffer setup");
    (runtime, control)
}

#[test]
fn empty_listing_from_nonproject_directory_creates_no_project() {
    let root = TestRuntimeRoot::new("native-cli-empty").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let output = cli(&root, &outside, &["--session-list"]);
    assert!(output.status.success(), "{:?}", text(&output));
    let (stdout, _) = text(&output);
    assert!(stdout.starts_with("ID"));
    assert!(stdout.contains("DIRECTORY"));
    assert!(!outside.join(".runyte").exists());
    assert!(!root.join("cache").exists());

    let missing_selector = cli(&root, &outside, &["--session-stop"]);
    assert!(!missing_selector.status.success());
    assert!(
        text(&missing_selector)
            .1
            .contains("requires an explicit workspace selector")
    );
    let restart = cli(&root, &outside, &["--session-restart", "project"]);
    assert!(!restart.status.success());
    assert!(text(&restart).1.contains("no session matches project"));
    assert!(!outside.join(".runyte").exists());
}

#[test]
fn restart_preflights_detached_policy_and_replaces_only_confirmed_real_host() {
    run_restart_fixture(RESTART_SUCCESS_FIXTURE, Duration::from_secs(80));
}

#[test]
fn restart_refuses_detached_policy_without_breakaway_job() {
    run_restart_fixture(RESTART_DENIAL_FIXTURE, Duration::from_secs(35));
}

#[test]
#[ignore = "reexecuted inside an admitted nested restart job"]
fn restart_success_fixture() {
    let outer = restart_fixture_outer_root();
    await_restart_fixture_admission(&outer);
    let _breakaway_job = enter_restart_breakaway_job();
    let root = TestRuntimeRoot::new_in("native-cli-restart", &outer).unwrap();
    restart_success_body(&root);
}

#[test]
#[ignore = "reexecuted inside a job that denies detached startup"]
fn restart_denial_fixture() {
    let outer = restart_fixture_outer_root();
    await_restart_fixture_admission(&outer);
    assert!(running_in_job());
    let root = TestRuntimeRoot::new_in("native-cli-restart-denial", &outer).unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let layout = layout(&root, "project");
    let mut original = start_host(&root, &layout);
    let location = layout.publication_location().unwrap();
    let before = location.read_ready().unwrap().unwrap();
    let project = layout.project_root().to_str().unwrap();
    let refused = cli(&root, &outside, &["--session-restart", project]);
    assert!(!refused.status.success());
    let failure = text(&refused).1;
    assert!(
        failure.contains("detached host preflight was denied")
            && failure.contains("CreateProcessW could not create the detached inheritance parent"),
        "unexpected restart refusal: {failure}"
    );
    assert!(original.0.try_wait().unwrap().is_none());
    assert_eq!(location.read_ready().unwrap().unwrap(), before);
}

fn restart_success_body(root: &TestRuntimeRoot) {
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let layout = layout(root, "project");
    let mut original = start_host(root, &layout);
    let location = layout.publication_location().unwrap();
    let before = location.read_ready().unwrap().unwrap();
    let before_pin = match PinnedProcess::open(before.process).unwrap() {
        PinResult::Pinned(process) => process,
        other => panic!("original host was not alive before restart: {other:?}"),
    };
    let project = layout.project_root().to_str().unwrap();
    let mut cleanup = RestartCleanup {
        root,
        cwd: &outside,
        selector: project,
        armed: true,
    };

    let first = cli(root, &outside, &["--session-restart", project]);
    assert!(first.status.success(), "{:?}", text(&first));
    assert!(original.0.try_wait().unwrap().is_some());
    assert!(!before_pin.is_alive().unwrap());
    let running = location.read_ready().unwrap().unwrap();
    assert_ne!(running.process, before.process);
    assert_eq!(running.project_root().unwrap(), layout.project_root());
    let running_pin = match PinnedProcess::open(running.process).unwrap() {
        PinResult::Pinned(process) => process,
        other => panic!("middle host was not alive after restart: {other:?}"),
    };

    let file = layout.project_root().join("note.txt");
    fs::write(&file, "original").unwrap();
    let (_runtime, control) = make_unsaved(&layout, &file);
    let refused = cli(root, &outside, &["--session-restart", project]);
    assert!(!refused.status.success());
    assert!(text(&refused).1.contains("unsaved"), "{:?}", text(&refused));
    assert_eq!(location.read_ready().unwrap().unwrap(), running);

    let forced = cli(root, &outside, &["--session-restart", "--force", project]);
    assert!(forced.status.success(), "{:?}", text(&forced));
    drop(control);
    assert!(!running_pin.is_alive().unwrap());
    let replaced = location.read_ready().unwrap().unwrap();
    assert_ne!(replaced.process, running.process);
    assert_eq!(fs::read_to_string(file).unwrap(), "original");
    let stopped = cli(root, &outside, &["--session-stop", project]);
    assert!(stopped.status.success(), "{:?}", text(&stopped));
    assert!(location.read_ready().unwrap().is_none());
    cleanup.armed = false;
}

#[test]
fn real_host_list_rename_and_normal_stop_from_nonproject_directory() {
    let root = TestRuntimeRoot::new("native-cli-normal").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let layout = layout(&root, "project");
    let mut host = start_host(&root, &layout);
    let project = layout.project_root().to_str().unwrap();

    let listing = cli(&root, &outside, &["--session-list"]);
    assert!(listing.status.success(), "{:?}", text(&listing));
    let (stdout, _) = text(&listing);
    assert!(stdout.contains(project) && stdout.contains("running"));

    let rename = cli(
        &root,
        &outside,
        &["--session-rename", project, "  new name  "],
    );
    assert!(rename.status.success(), "{:?}", text(&rename));
    let renamed = cli(&root, &outside, &["--session-list"]);
    assert!(renamed.status.success(), "{:?}", text(&renamed));
    assert!(text(&renamed).0.contains("new-name"));

    let stop = cli(&root, &outside, &["--session-stop", "new-name"]);
    assert!(stop.status.success(), "{:?}", text(&stop));
    assert!(
        host.0.try_wait().unwrap().is_some(),
        "CLI returned before host exit"
    );
    remember(&layout).unwrap();
    let stopped_rename = cli(&root, &outside, &["--session-rename", project, "retired"]);
    assert!(
        stopped_rename.status.success(),
        "{:?}",
        text(&stopped_rename)
    );
    let stopped = cli(&root, &outside, &["--session-list"]);
    assert!(stopped.status.success(), "{:?}", text(&stopped));
    let (stopped_text, _) = text(&stopped);
    assert!(stopped_text.contains("retired") && stopped_text.contains("stopped"));
    let clean = cli(&root, &outside, &["--session-clean"]);
    assert!(clean.status.success(), "{:?}", text(&clean));
    assert!(text(&clean).0.contains("forgot 1 stopped session"));
    assert_eq!(
        NameStore::read_existing(layout.state_root(), &workspace_id(layout.project_root()))
            .unwrap()
            .as_deref(),
        Some("retired")
    );
    let forgotten = cli(&root, &outside, &["--session-list"]);
    assert!(forgotten.status.success());
    assert!(!text(&forgotten).0.contains("retired"));
    assert!(!outside.join(".runyte").exists());
}

#[tokio::test]
async fn live_worktree_host_stops_before_git_removal_and_exact_history_cleanup() {
    let root = TestRuntimeRoot::new("native-worktree-host-cascade").unwrap();
    let main = layout(&root, "main");
    let main_root = main.project_root();
    let git = |cwd: &Path, arguments: &[&str]| {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("empty-gitconfig"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(main_root, &["init", "-q"]);
    git(main_root, &["config", "core.autocrlf", "false"]);
    fs::write(main_root.join(".gitignore"), ".runyte/\n").unwrap();
    fs::write(main_root.join("note.txt"), "base\n").unwrap();
    git(main_root, &["add", ".gitignore", "note.txt"]);
    git(
        main_root,
        &[
            "-c",
            "user.name=Runyte Test",
            "-c",
            "user.email=runyte@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "base",
        ],
    );

    let provider = GitCliProvider::from_environment().expect("Git is installed for this test");
    let repository = provider.discover(main_root).unwrap().unwrap();
    let linked_path = root.join("linked");
    provider
        .create_worktree(
            &repository,
            &WorktreeCreate {
                destination: linked_path.clone(),
                start: "HEAD".to_owned(),
                new_branch: Some("cascaded".to_owned()),
                upstream: None,
            },
        )
        .unwrap();
    let linked = layout(&root, "linked");
    let linked_path = linked.project_root().to_owned();
    let mut host = start_host(&root, &linked);
    remember(&linked).unwrap();
    let branch_plan = provider
        .prepare_branch_deletion_through(&repository, "cascaded", &linked_path)
        .unwrap();
    let removal_plan = provider
        .prepare_worktree_removal(&repository, &linked_path)
        .unwrap();

    let (service, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        main.discovery_scope().clone(),
        Some(main.read_location()),
        PathBuf::from(".runyte"),
    )
    .unwrap();
    service
        .try_inspect_worktree_teardown(41, &linked_path)
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
        .await
        .unwrap()
        .unwrap();
    let WorkspaceEvent::WorktreeInspected { result, .. } = event else {
        panic!("worktree inspection returned {event:?}");
    };
    let row = (*result).unwrap().expect("real host must be visible");
    assert!(row.running);
    assert_eq!(row.unsaved_buffers, Some(0));
    service
        .try_prepare_worktree_teardown(42, &linked_path, Some(row.selection()))
        .unwrap();
    let prepared = loop {
        let event = tokio::time::timeout(Duration::from_secs(12), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            WorkspaceEvent::WorktreePrepared { result, .. } => break (*result).unwrap(),
            WorkspaceEvent::ControlWarnings { .. } => {}
            other => panic!("worktree stop returned {other:?}"),
        }
    };
    assert_eq!(
        prepared
            .stopped_session()
            .map(|row| row.project_root.as_path()),
        Some(linked_path.as_path())
    );
    assert!(
        host.0.try_wait().unwrap().is_some(),
        "host still ran after stop"
    );
    let lease = prepared.into_lease();
    lease.verify_live_identity().unwrap();
    provider
        .remove_worktree_guarded(&repository, &removal_plan, DeletionAuthorization::Typed)
        .unwrap();
    assert!(!linked_path.exists());
    service
        .try_finish_worktree_teardown(42, &linked_path, lease)
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        WorkspaceEvent::WorktreeFinalized {
            generation: 42,
            result: Ok(true),
            ..
        }
    ));
    let history_path = linked
        .cache_root()
        .unwrap()
        .unwrap()
        .join("workspaces.json");
    let history: serde_json::Value =
        serde_json::from_slice(&fs::read(history_path).unwrap()).unwrap();
    assert_eq!(history.as_array().unwrap().len(), 0);

    provider
        .delete_branch_guarded(&repository, &branch_plan, DeletionAuthorization::Typed)
        .unwrap();
    assert!(
        provider
            .worktrees(&repository)
            .unwrap()
            .iter()
            .all(|worktree| worktree.path != linked_path)
    );
    assert!(
        provider
            .branches(&repository)
            .unwrap()
            .iter()
            .all(|branch| branch.name != "cascaded")
    );
    owner.shutdown().await.unwrap();
}

#[test]
fn protected_host_refuses_normal_stop_and_force_confirms_exit() {
    let root = TestRuntimeRoot::new("native-cli-force").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let layout = layout(&root, "project");
    let file = layout.project_root().join("note.txt");
    fs::write(&file, "original").unwrap();
    let mut host = start_host(&root, &layout);
    let (_runtime, control) = make_unsaved(&layout, &file);

    let project = layout.project_root().to_str().unwrap();
    let refused = cli(&root, &outside, &["--session-stop", project]);
    assert!(!refused.status.success());
    assert!(text(&refused).1.contains("unsaved"), "{:?}", text(&refused));
    assert!(host.0.try_wait().unwrap().is_none());

    let forced = cli(&root, &outside, &["--session-stop", "--force", project]);
    assert!(forced.status.success(), "{:?}", text(&forced));
    drop(control);
    assert!(
        host.0.try_wait().unwrap().is_some(),
        "force returned before process exit"
    );
    assert_eq!(fs::read_to_string(file).unwrap(), "original");
    assert!(!outside.join(".runyte").exists());
}

#[test]
fn stop_all_continues_after_protected_refusal_and_reports_partial_success() {
    let root = TestRuntimeRoot::new("native-cli-stop-all").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let protected = layout(&root, "a-protected");
    let clean = layout(&root, "z-clean");
    let file = protected.project_root().join("note.txt");
    fs::write(&file, "original").unwrap();
    let mut protected_host = start_host(&root, &protected);
    let mut clean_host = start_host(&root, &clean);
    let (_runtime, control) = make_unsaved(&protected, &file);

    let partial = cli(&root, &outside, &["--session-stop-all"]);
    assert!(!partial.status.success());
    let (_, failure) = text(&partial);
    assert!(
        failure.contains("stopped 1 of 2 running sessions"),
        "{failure}"
    );
    assert!(failure.contains("1 failed") && failure.contains("unsaved"));
    assert!(protected_host.0.try_wait().unwrap().is_none());
    assert!(clean_host.0.try_wait().unwrap().is_some());

    let forced = cli(
        &root,
        &outside,
        &[
            "--session-stop",
            "--force",
            protected.project_root().to_str().unwrap(),
        ],
    );
    assert!(forced.status.success(), "{:?}", text(&forced));
    drop(control);
    assert!(protected_host.0.try_wait().unwrap().is_some());
    assert_eq!(fs::read_to_string(file).unwrap(), "original");
}

#[test]
fn incomplete_registry_refuses_list_and_clean_without_history_mutation() {
    let root = TestRuntimeRoot::new("native-cli-incomplete").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let remembered_stopped = layout(&root, "stopped");
    let layout = layout(&root, "project");
    let mut host = start_host(&root, &layout);
    remember(&layout).unwrap();
    remember(&remembered_stopped).unwrap();
    let malformed = remembered_stopped
        .endpoint_directory()
        .join("endpoint.json");
    root.atomic_write_private(
        remembered_stopped.endpoint_directory(),
        OsStr::new("endpoint.json"),
        b"malformed",
    )
    .unwrap();

    let listing = cli(&root, &outside, &["--session-list"]);
    assert!(!listing.status.success());
    assert!(
        listing.stdout.is_empty(),
        "incomplete inventory printed a successful table"
    );
    let cleaning = cli(&root, &outside, &["--session-clean"]);
    assert!(!cleaning.status.success());
    assert_eq!(fs::read(&malformed).unwrap(), b"malformed");
    fs::remove_file(malformed).unwrap();

    let complete = cli(&root, &outside, &["--session-list"]);
    assert!(complete.status.success(), "{:?}", text(&complete));
    assert!(
        text(&complete)
            .0
            .contains(layout.project_root().to_str().unwrap())
    );
    let stop = cli(
        &root,
        &outside,
        &["--session-stop", layout.project_root().to_str().unwrap()],
    );
    assert!(stop.status.success(), "{:?}", text(&stop));
    assert!(host.0.try_wait().unwrap().is_some());
    let retained = cli(&root, &outside, &["--session-list"]);
    assert!(retained.status.success(), "{:?}", text(&retained));
    let (rows, _) = text(&retained);
    assert!(rows.contains("stopped") && rows.contains(layout.project_root().to_str().unwrap()));
    assert!(rows.contains(remembered_stopped.project_root().to_str().unwrap()));
}

#[test]
fn hidden_isolated_publications_of_one_project_remain_two_live_rows() {
    let root = TestRuntimeRoot::new("native-cli-isolated").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let selected = layout(&root, "project");
    let other_state = root.join("state-b");
    let other_cache = root.join("cache-b");
    let other = ResolvedLayout::resolve(LocationInputs {
        project_root: selected.project_root().to_owned(),
        state_root: other_state.clone(),
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            cache_home: Some(other_cache.clone()),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    })
    .unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    let config = root.join("config/isolated.yaml");
    fs::write(
        &config,
        format!(
            "workspace:\n  state: {}\n",
            serde_json::to_string(other_state.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let mut first = start_host(&root, &selected);
    let mut second = start_host_in(&root, &other, &other_cache, Some(&config));
    let project = selected.project_root().to_str().unwrap();

    let ordinary = cli(&root, &outside, &["--session-list"]);
    assert!(ordinary.status.success(), "{:?}", text(&ordinary));
    assert_eq!(
        text(&ordinary)
            .0
            .lines()
            .filter(|line| line.contains(project))
            .count(),
        1
    );
    let broad = cli(&root, &outside, &["--session-list", "--include-hidden"]);
    assert!(broad.status.success(), "{:?}", text(&broad));
    assert_eq!(
        text(&broad)
            .0
            .lines()
            .filter(|line| line.contains(project))
            .count(),
        2
    );

    let stopped = cli(&root, &outside, &["--session-stop-all", "--include-hidden"]);
    assert!(stopped.status.success(), "{:?}", text(&stopped));
    assert!(text(&stopped).0.contains("stopped 2 sessions"));
    assert!(first.0.try_wait().unwrap().is_some());
    assert!(second.0.try_wait().unwrap().is_some());
    assert!(!outside.join(".runyte").exists());
}

#[test]
fn restart_refuses_ambiguous_live_id_prefix() {
    let root = TestRuntimeRoot::new("native-cli-restart-ambiguous").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    // Seventeen distinct roots guarantee two SHA-256 workspace IDs have the
    // same first hex digit; only those two roots are started as real hosts.
    let layouts = (0..17)
        .map(|index| layout(&root, &format!("project-{index}")))
        .collect::<Vec<_>>();
    let ids = layouts
        .iter()
        .map(|layout| workspace_id(layout.project_root()))
        .collect::<Vec<_>>();
    let (first_index, second_index) = (0..ids.len())
        .find_map(|second| {
            (0..second)
                .find(|first| ids[*first][..1] == ids[second][..1])
                .map(|first| (first, second))
        })
        .unwrap();
    let first_layout = &layouts[first_index];
    let second_layout = &layouts[second_index];
    let prefix = &ids[first_index][..1];
    let mut first = start_host(&root, first_layout);
    let mut second = start_host(&root, second_layout);
    let listing = cli(&root, &outside, &["--session-list"]);
    assert!(listing.status.success(), "{:?}", text(&listing));
    assert!(
        text(&listing)
            .0
            .contains(first_layout.project_root().to_str().unwrap())
    );
    assert!(
        text(&listing)
            .0
            .contains(second_layout.project_root().to_str().unwrap())
    );

    let ambiguous = cli(&root, &outside, &["--session-restart", prefix]);
    assert!(!ambiguous.status.success());
    assert!(
        text(&ambiguous)
            .1
            .contains("matches multiple native publications or history entries"),
        "{:?}",
        text(&ambiguous)
    );
    assert!(first.0.try_wait().unwrap().is_none());
    assert!(second.0.try_wait().unwrap().is_none());
    for layout in [first_layout, second_layout] {
        assert!(
            layout
                .publication_location()
                .unwrap()
                .read_ready()
                .unwrap()
                .is_some()
        );
        let stopped = cli(
            &root,
            &outside,
            &["--session-stop", layout.project_root().to_str().unwrap()],
        );
        assert!(stopped.status.success(), "{:?}", text(&stopped));
    }
    assert!(first.0.try_wait().unwrap().is_some());
    assert!(second.0.try_wait().unwrap().is_some());
}

fn manager_key(app: &mut App, code: KeyCode) {
    app.handle_key(KeyStroke::new(code, Modifiers::NONE))
        .unwrap();
}

fn manager_overlay(app: &App) -> runyte::snapshot::OverlaySnapshot {
    app.overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Sessions"))
        .expect("session manager is open")
}

fn select_manager_name(app: &mut App, name: &str) {
    for _ in 0..manager_overlay(app).rows.len() {
        let overlay = manager_overlay(app);
        if overlay
            .selected
            .and_then(|index| overlay.rows.get(index))
            .is_some_and(|row| row.label.contains(name))
        {
            return;
        }
        manager_key(app, KeyCode::Down);
    }
    panic!("manager did not select {name}");
}

fn select_manager_action(app: &mut App, label: &str) {
    manager_key(app, KeyCode::Tab);
    for _ in 0..5 {
        let menu = app
            .overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == runyte::snapshot::OverlayKind::BufferActions)
            .expect("session action menu is open");
        if menu
            .selected
            .and_then(|index| menu.rows.get(index))
            .is_some_and(|row| row.label == label)
        {
            manager_key(app, KeyCode::Enter);
            return;
        }
        manager_key(app, KeyCode::Down);
    }
    panic!("session action {label} was absent");
}

fn apply_native_event(
    runtime: &tokio::runtime::Runtime,
    app: &mut App,
    events: &mut tokio::sync::mpsc::Receiver<WorkspaceEvent>,
    predicate: impl Fn(&WorkspaceEvent) -> bool,
) {
    for _ in 0..16 {
        let event = runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(6), events.recv()).await })
            .expect("native session manager service did not answer")
            .expect("native session manager service ended");
        let done = predicate(&event);
        app.apply_workspace_event(event);
        if done {
            return;
        }
    }
    panic!("native session manager did not produce expected event");
}

#[test]
fn manager_controls_exact_real_publication_and_waits_for_force_exit() {
    let root = TestRuntimeRoot::new("native-manager-real-host").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let selected = layout(&root, "project");
    let other_state = root.join("state-b");
    let other_cache = root.join("cache-b");
    let other = ResolvedLayout::resolve(LocationInputs {
        project_root: selected.project_root().to_owned(),
        state_root: other_state.clone(),
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            cache_home: Some(other_cache.clone()),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    })
    .unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    let isolated_config = root.join("config/isolated.yaml");
    fs::write(
        &isolated_config,
        format!(
            "workspace:\n  state: {}\n",
            serde_json::to_string(other_state.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let mut protected = start_host(&root, &selected);
    let rename = cli(
        &root,
        &outside,
        &[
            "--session-rename",
            selected.project_root().to_str().unwrap(),
            "protected",
        ],
    );
    assert!(rename.status.success(), "{:?}", text(&rename));
    let file = selected.project_root().join("note.txt");
    fs::write(&file, "original").unwrap();
    let (_control_runtime, control) = make_unsaved(&selected, &file);
    let mut unrelated = start_host_in(&root, &other, &other_cache, Some(&isolated_config));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (service, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        selected.discovery_scope().clone(),
        Some(selected.read_location()),
        std::path::PathBuf::from(".runyte"),
    )
    .unwrap();
    let mut app = App::new_in_project(Config::default(), None, selected.project_root()).unwrap();
    app.attach_workspace_service(service.clone());
    app.execute(parse_named_command("session-list", None).unwrap())
        .unwrap();
    service.try_refresh(1, true).unwrap();
    let selected_identity = std::cell::RefCell::new(None);
    apply_native_event(&runtime, &mut app, &mut events, |event| {
        if let WorkspaceEvent::Refreshed {
            result: Ok(rows), ..
        } = event
            && rows.len() == 2
        {
            *selected_identity.borrow_mut() = rows
                .iter()
                .find(|row| row.name.as_deref() == Some("protected"))
                .map(|row| row.selection());
            true
        } else {
            false
        }
    });
    let selected_identity = selected_identity
        .into_inner()
        .expect("protected publication is listed");
    let overlay = manager_overlay(&app);
    assert_eq!(overlay.rows.len(), 2);
    assert!(!overlay.rows.iter().any(|row| row.label.contains('*')));
    assert!(
        !overlay
            .actions
            .iter()
            .any(|action| action.label == "attach")
    );

    select_manager_name(&mut app, "protected");
    apply_native_event(
        &runtime,
        &mut app,
        &mut events,
        |event| matches!(event, WorkspaceEvent::Previewed { selection, result: Ok(_), .. } if selection == &selected_identity),
    );
    let preview = format!("{:?}", manager_overlay(&app).preview);
    assert!(preview.contains("Unsaved     1"), "{preview}");

    select_manager_action(&mut app, "Rename");
    for _ in "protected".chars() {
        manager_key(&mut app, KeyCode::Backspace);
    }
    for character in "managed".chars() {
        manager_key(&mut app, KeyCode::Char(character));
    }
    manager_key(&mut app, KeyCode::Enter);
    apply_native_event(
        &runtime,
        &mut app,
        &mut events,
        |event| matches!(event, WorkspaceEvent::Renamed { selection: Some(selection), result: Ok(()), .. } if selection == &selected_identity),
    );
    assert_eq!(
        NameStore::read_existing(
            selected.state_root(),
            &workspace_id(selected.project_root())
        )
        .unwrap(),
        Some("managed".to_owned())
    );
    apply_native_event(&runtime, &mut app, &mut events, |event| {
        matches!(event, WorkspaceEvent::Refreshed { result: Ok(_), .. })
    });
    select_manager_name(&mut app, "managed");

    select_manager_action(&mut app, "Close");
    apply_native_event(
        &runtime,
        &mut app,
        &mut events,
        |event| matches!(event, WorkspaceEvent::Stopped { selection: Some(selection), result: Err(_), .. } if selection == &selected_identity),
    );
    assert!(protected.0.try_wait().unwrap().is_none());
    assert!(unrelated.0.try_wait().unwrap().is_none());

    select_manager_action(&mut app, "Force close");
    manager_key(&mut app, KeyCode::Enter);
    apply_native_event(
        &runtime,
        &mut app,
        &mut events,
        |event| matches!(event, WorkspaceEvent::Stopped { selection: Some(selection), result: Ok(()), .. } if selection == &selected_identity),
    );
    drop(control);
    assert!(protected.0.try_wait().unwrap().is_some());
    assert!(unrelated.0.try_wait().unwrap().is_none());
    runtime.block_on(owner.shutdown()).unwrap();
}

#[test]
fn captured_manager_rename_refuses_real_replacement_at_same_location() {
    let root = TestRuntimeRoot::new("native-manager-stale-host").unwrap();
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let layout = layout(&root, "project");
    let mut original = start_host(&root, &layout);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (service, mut owner, mut events) = WorkspaceServiceOwner::spawn(
        layout.discovery_scope().clone(),
        Some(layout.read_location()),
        std::path::PathBuf::from(".runyte"),
    )
    .unwrap();
    let mut app = App::new_in_project(Config::default(), None, layout.project_root()).unwrap();
    app.attach_workspace_service(service.clone());
    app.execute(parse_named_command("session-list", None).unwrap())
        .unwrap();
    let old_selection = std::cell::RefCell::new(None);
    apply_native_event(&runtime, &mut app, &mut events, |event| {
        if let WorkspaceEvent::Refreshed {
            result: Ok(rows), ..
        } = event
            && rows.len() == 1
        {
            *old_selection.borrow_mut() = Some(rows[0].selection());
            true
        } else {
            false
        }
    });
    let old_selection = old_selection.into_inner().unwrap();
    select_manager_action(&mut app, "Rename");

    let stopped = cli(
        &root,
        &outside,
        &["--session-stop", layout.project_root().to_str().unwrap()],
    );
    assert!(stopped.status.success(), "{:?}", text(&stopped));
    assert!(original.0.try_wait().unwrap().is_some());
    let mut replacement = start_host(&root, &layout);
    let id = workspace_id(layout.project_root());
    let name_before = NameStore::read_existing(layout.state_root(), &id).unwrap();
    service.try_poll().unwrap();
    let new_selection = std::cell::RefCell::new(None);
    apply_native_event(&runtime, &mut app, &mut events, |event| {
        if let WorkspaceEvent::Polled { result: Ok(rows) } = event
            && rows.len() == 1
        {
            *new_selection.borrow_mut() = Some(rows[0].selection());
            true
        } else {
            false
        }
    });
    assert_ne!(old_selection, new_selection.into_inner().unwrap());

    for _ in 0..64 {
        manager_key(&mut app, KeyCode::Backspace);
    }
    for character in "stale-change".chars() {
        manager_key(&mut app, KeyCode::Char(character));
    }
    manager_key(&mut app, KeyCode::Enter);
    apply_native_event(
        &runtime,
        &mut app,
        &mut events,
        |event| matches!(event, WorkspaceEvent::Renamed { selection: Some(selection), result: Err(_), .. } if selection == &old_selection),
    );
    assert!(replacement.0.try_wait().unwrap().is_none());
    assert_eq!(
        NameStore::read_existing(layout.state_root(), &id).unwrap(),
        name_before
    );
    runtime.block_on(owner.shutdown()).unwrap();
}
