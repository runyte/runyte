// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    private_storage::Directory,
    protocol::{ClientRequest, FeatureGroup, HostResponse},
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{Publication, RegistryRecord, RegistrySet},
        windows_transport::{LocalServer, ServerEvent},
    },
};
use std::{
    collections::HashMap,
    ffi::OsStr,
    io::Read,
    os::windows::io::{FromRawHandle, OwnedHandle},
    os::windows::process::CommandExt,
    process::Stdio,
};
use windows_sys::Win32::{
    Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, WAIT_OBJECT_0},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
            JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, GetCurrentProcess, OpenProcess, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
        },
    },
};

const FIXTURE: &str = "workspace::windows_startup::tests::native_startup_fixture";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn location(root: &Path) -> EndpointLocation {
    fs::create_dir_all(root.join("project")).unwrap();
    EndpointLocation::new(
        &root.join("project"),
        root.join("endpoint"),
        RegistrySet::open_fixture(&[root.join("registry")]).unwrap(),
    )
    .unwrap()
}

fn command(root: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "--exact",
            FIXTURE,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(root)
        .env("RUNYTE_STARTUP_FIXTURE_ROOT", root)
        .env("RUNYTE_STARTUP_FIXTURE_MODE", mode)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"));
    command
}

fn duplicate(handle: HANDLE) -> OwnedHandle {
    let mut value = std::ptr::null_mut();
    assert_ne!(
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle,
                GetCurrentProcess(),
                &mut value,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        },
        0
    );
    unsafe { OwnedHandle::from_raw_handle(value) }
}

struct FixtureOwner {
    job: OwnedHandle,
    leader: OwnedHandle,
    armed: bool,
}
impl FixtureOwner {
    fn new(child: &StartupChild) -> Self {
        Self::from_handles(child.job_handle(), child.handle())
    }
    fn from_handles(job: HANDLE, leader: HANDLE) -> Self {
        Self {
            job: duplicate(job),
            leader: duplicate(leader),
            armed: true,
        }
    }
    fn leader_exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.leader.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }
    fn release(mut self) {
        self.armed = false;
    }
}

const CONTROLLED_FIXTURE: &str =
    "workspace::windows_startup::tests::native_controlled_startup_fixture";
const CONTROLLED_CASE: &str = "RUNYTE_STARTUP_CONTROLLED_CASE";

/// Cargo and other supervisors may place libtest in a restrictive job. Every
/// spawning case runs in its own normally-created, owned fixture process; only
/// that isolated child joins an allowed inner job. Real startup still requests
/// breakaway and remains inside the fixture's restrictive outer cleanup job.
fn run_in_controlled_job(case: &str) -> bool {
    if std::env::var(CONTROLLED_CASE).as_deref() == Ok(case) {
        return false;
    }
    let root = TestRuntimeRoot::new("controlled-startup").unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "--exact",
            CONTROLLED_FIXTURE,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(root.path())
        .env(CONTROLLED_CASE, case)
        .env("RUNYTE_STARTUP_FIXTURE_ROOT", root.path())
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"));
    let input: OwnedHandle = fs::OpenOptions::new()
        .read(true)
        .open("NUL")
        .unwrap()
        .into();
    let output: OwnedHandle = fs::OpenOptions::new()
        .write(true)
        .open("NUL")
        .unwrap()
        .into();
    let error: OwnedHandle = fs::OpenOptions::new()
        .write(true)
        .open("NUL")
        .unwrap()
        .into();
    let mut child =
        crate::windows_process::spawn_with_stdio(&command, [input, output, error]).unwrap();
    let owner =
        FixtureOwner::from_handles(child.fixture_job_handle(), child.fixture_process_handle());
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    // This guard observes whole-job completion before any fixture storage is
    // removed, including when the child failed before installing diagnostics.
    drop(owner);
    let mut diagnostic = Vec::new();
    if let Ok(file) = fs::File::open(root.join("controlled-failure.txt")) {
        file.take(4096).read_to_end(&mut diagnostic).unwrap();
    }
    assert!(
        status.is_some_and(|status| status.success()),
        "controlled startup case {case} failed ({status:?}): {}",
        String::from_utf8_lossy(&diagnostic),
    );
    true
}

fn current_job_diagnostic() -> String {
    let mut member = 0;
    let membership =
        unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut member) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    let queried = unsafe {
        QueryInformationJobObject(
            std::ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
            std::ptr::null_mut(),
        )
    };
    format!(
        "membership_query={membership} in_job={member} limits_query={queried} immediate_flags={:#x}",
        limits.BasicLimitInformation.LimitFlags
    )
}

#[test]
#[ignore = "compiled wrapper with controlled native startup job policy"]
fn native_controlled_startup_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_STARTUP_FIXTURE_ROOT").unwrap());
    let initial = current_job_diagnostic();
    std::panic::set_hook(Box::new(move |info| {
        let detail = bounded_detail(&format!(
            "initial {initial}; current {}; {info}",
            current_job_diagnostic()
        ));
        let _ = fs::write(root.join("controlled-failure.txt"), detail);
    }));
    let _inner = enter_breakaway_inner_job();
    match std::env::var(CONTROLLED_CASE).unwrap().as_str() {
        "successful_ready" => {
            successful_ready_releases_only_its_job_and_survives_startup_owner_drop()
        }
        "cancellation" => cancellation_and_timeout_terminate_the_whole_provisional_job(),
        "invalid_readiness" => startup_does_not_release_on_early_exit_silent_or_invalid_welcome(),
        "external_winner" => raced_external_winner_is_retained_and_owned_loser_is_terminated(),
        "occupied" => {
            occupied_preflight_never_spawns_and_winner_commit_rechecks_deadline_and_liveness()
        }
        "provisional_refusal" => {
            provisional_launcher_inside_a_nonbreakaway_job_is_refused_without_weakening_it()
        }
        "released_launcher" => {
            released_host_can_start_another_host_that_survives_its_launcher_process()
        }
        "nested_policy" => allowed_inner_job_does_not_escape_a_restrictive_outer_job(),
        "missing_executable" => {
            actual_spawn_missing_executable_preserves_typed_upgrade_diagnostic()
        }
        other => panic!("unknown controlled startup fixture case {other}"),
    }
}
impl Drop for FixtureOwner {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        unsafe {
            TerminateJobObject(self.job.as_raw_handle(), 1);
        }
        let deadline = std::time::Instant::now() + CLEANUP_BUDGET;
        let empty = loop {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
            let queried = unsafe {
                QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                )
            };
            if queried == 0 {
                break false;
            }
            // Accounting can reach zero before the retained leader process
            // object becomes signaled. Storage/proof assertions require both.
            if info.ActiveProcesses == 0 && self.leader_exited() {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        if !std::thread::panicking() {
            assert!(
                empty,
                "compiled startup fixture job did not empty before cleanup"
            );
        }
    }
}

// This guard is only for the compiled fixture that intentionally escapes its
// launcher's job. It retains a termination handle to the authenticated exact
// process; no production lifecycle code acquires this authority by PID alone.
struct ReleasedFixture(OwnedHandle);
impl ReleasedFixture {
    fn new(peer: &PinnedProcess) -> Self {
        let handle = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                0,
                peer.identity().pid,
            )
        };
        assert!(!handle.is_null());
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        assert_ne!(
            unsafe {
                CompareObjectHandles(handle.as_raw_handle(), peer.as_handle().as_raw_handle())
            },
            0
        );
        Self(handle)
    }
}
impl Drop for ReleasedFixture {
    fn drop(&mut self) {
        unsafe {
            TerminateProcess(self.0.as_raw_handle(), 1);
            WaitForSingleObject(self.0.as_raw_handle(), 5000);
        }
    }
}

fn enter_breakaway_inner_job() -> OwnedHandle {
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    assert!(!raw.is_null());
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    // The parent's restrictive, kill-on-close fixture job remains the lifetime
    // owner. This additional inner job only permits the explicit startup escape.
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    assert_ne!(
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        },
        0
    );
    assert_ne!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) },
        0
    );
    job
}

fn welcome() -> HostResponse {
    HostResponse::Welcome {
        protocol: crate::protocol::VERSION,
        pid: std::process::id(),
        features: vec![
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ],
        host_version: crate::protocol::CLIENT_VERSION.into(),
    }
}

#[test]
#[ignore = "compiled child fixture for owned native startup"]
fn native_startup_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_STARTUP_FIXTURE_ROOT").unwrap());
    let mode = std::env::var("RUNYTE_STARTUP_FIXTURE_MODE").unwrap();
    let panic_file = root.join("fixture-panic.txt");
    std::panic::set_hook(Box::new(move |info| {
        let _ = fs::write(&panic_file, bounded_detail(&info.to_string()));
    }));
    match mode.as_str() {
        "exit" => std::process::exit(17),
        "idle" => {
            std::thread::sleep(Duration::from_secs(20));
            return;
        }
        "constrained-launcher" => {
            // This fixture itself is inside an unreleased no-breakaway job.
            assert!(StartupChild::spawn(&command(&root, "idle")).is_err());
            fs::write(root.join("breakaway-refused"), b"refused").unwrap();
            return;
        }
        "released-acceptance" => {
            let _inner = enter_breakaway_inner_job();
            runtime().block_on(released_launcher_scenario(&root));
            fs::write(root.join("acceptance-complete"), b"passed").unwrap();
            return;
        }
        "nested-policy" => {
            // An allowed immediate job does not promise escape from the
            // restrictive outer job owned by this fixture's parent.
            let _inner = enter_breakaway_inner_job();
            runtime().block_on(async {
                let next_root = root.join("next");
                fs::create_dir(&next_root).unwrap();
                let endpoint = location(&next_root);
                let child = StartupChild::spawn(&command(&next_root, "serve")).unwrap();
                wait_ready(&endpoint, child, Instant::now() + READINESS_BUDGET)
                    .await
                    .unwrap();
                fs::write(root.join("next-launched"), b"ready").unwrap();
                tokio::time::sleep(Duration::from_secs(15)).await;
            });
            return;
        }
        "descendants" | "descendant-server" => {
            let mut child = command(
                &root,
                if mode == "descendants" {
                    "idle"
                } else {
                    "serve"
                },
            );
            let mut child = child
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            fs::write(root.join("descendant-pid"), child.id().to_string()).unwrap();
            child.wait().unwrap();
            return;
        }
        _ => {}
    }
    runtime().block_on(async {
        let endpoint = location(&root);
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        if mode == "incompatible" {
            let mut metadata = server.metadata().clone();
            metadata.protocol += 1;
            Directory::open_existing(&root.join("endpoint"), true)
                .unwrap()
                .atomic_write(OsStr::new("endpoint.json"), &serde_json::to_vec(&metadata).unwrap())
                .unwrap();
        }
        // bind() first publishes a compatible ready record. Tests replacing
        // its protocol must not race that initial publication's existence.
        fs::write(root.join("configured-ready"), b"configured").unwrap();
        let mut responses = HashMap::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if mode == "released-launcher" && root.join("launch-next").exists() {
                let next_root = root.join("next");
                fs::create_dir(&next_root).unwrap();
                let endpoint = location(&next_root);
                let child = StartupChild::spawn(&command(&next_root, "serve")).unwrap();
                let owner = FixtureOwner::new(&child);
                wait_ready(&endpoint, child, Instant::now() + READINESS_BUDGET).await.unwrap();
                fs::write(root.join("next-launched"), b"ready").unwrap();
                // Retain cleanup ownership until the outer fixture explicitly
                // acknowledges its own exact-process termination handle.
                await_marker(&root.join("next-owned")).await;
                owner.release();
                break;
            }
            let event = tokio::select! {
                result = timeout_at(deadline, server.recv()) => match result {
                    Ok(Some(event)) => event,
                    _ => break,
                },
                _ = tokio::time::sleep(Duration::from_millis(25)), if mode == "released-launcher" => continue,
            };
            match event {
                ServerEvent::Connected {
                    id,
                    responses: sender,
                    ..
                } => {
                    if mode != "silent" {
                        let mut response = welcome();
                        if mode == "wrong-welcome"
                            && let HostResponse::Welcome { pid, .. } = &mut response
                        {
                            *pid = pid.wrapping_add(1);
                        }
                        sender.send(response).await.unwrap();
                    }
                    responses.insert(id, sender);
                }
                ServerEvent::Disconnected { id } => {
                    responses.remove(&id);
                }
                ServerEvent::Request {
                    id,
                    request: ClientRequest::Shutdown | ClientRequest::ForceShutdown,
                } => {
                    responses
                        .get(&id)
                        .unwrap()
                        .send(HostResponse::ShuttingDown)
                        .await
                        .unwrap();
                    break;
                }
                _ => {}
            }
        }
        responses.clear();
        server.shutdown().await.unwrap();
    });
}

fn fixture() -> (TestRuntimeRoot, EndpointLocation) {
    let root = TestRuntimeRoot::new("native-startup").unwrap();
    let endpoint = location(root.path());
    (root, endpoint)
}

async fn await_marker(path: &Path) {
    await_marker_until(path, Instant::now() + Duration::from_secs(5)).await;
}

async fn await_marker_until(path: &Path, deadline: Instant) {
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture marker {} did not appear",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[test]
fn command_recipe_preserves_literal_arguments_explicit_context_and_config_environment() {
    let (root, endpoint) = fixture();
    let nested = endpoint.project_root().join("nested 雪");
    fs::create_dir(&nested).unwrap();
    let mut startup = HostStartup::new(std::env::current_exe().unwrap());
    startup.working_directory = Some(nested.clone());
    startup.config = Some(root.join("config space.toml"));
    startup.log = Some(root.join("diagnostic.log"));
    startup.targets = vec![root.join("literal $; ' quote.txt"), root.join("tail\\")];
    startup.verbosity = 2;
    startup.env = vec![
        (
            "RUNYTE_LITERAL".into(),
            Some("$not-expanded; 'quoted'".into()),
        ),
        ("RUNYTE_REMOVE".into(), None),
    ];
    let command = startup.command(&endpoint).unwrap();
    let args = command.get_args().collect::<Vec<_>>();
    assert_eq!(
        args[..3],
        [
            OsStr::new("--serve"),
            OsStr::new("--detached-host"),
            OsStr::new("--project-root")
        ]
    );
    assert_eq!(args[3], endpoint.project_root().as_os_str());
    assert_eq!(args[5], root.join("config space.toml").as_os_str());
    assert_eq!(args[6..8], [OsStr::new("-v"), OsStr::new("-v")]);
    assert_eq!(args[10], OsStr::new("--"));
    assert_eq!(args[11], startup.targets[0].as_os_str());
    assert_eq!(args[12], startup.targets[1].as_os_str());
    assert_eq!(
        command.get_current_dir().unwrap().canonicalize().unwrap(),
        nested.canonicalize().unwrap()
    );
    assert!(
        command
            .get_envs()
            .any(|(name, value)| name == "RUNYTE_REMOVE" && value.is_none())
    );
    startup.working_directory = Some(root.path().to_owned());
    assert!(startup.command(&endpoint).is_err());
    startup.executable = root.join("missing.exe");
    assert!(
        startup
            .command(&endpoint)
            .unwrap_err()
            .downcast_ref::<UnavailableStartupExecutable>()
            .is_some()
    );
}

#[test]
fn successful_ready_releases_only_its_job_and_survives_startup_owner_drop() {
    if run_in_controlled_job("successful_ready") {
        return;
    }
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let child = StartupChild::spawn(&command(root.path(), "serve")).unwrap();
        let owner = FixtureOwner::new(&child);
        let started = wait_ready(&endpoint, child, Instant::now() + READINESS_BUDGET)
            .await
            .unwrap();
        assert_eq!(started.disposition(), StartDisposition::Started);
        assert!(!owner.leader_exited());
        assert!(started.peer().is_alive().unwrap());
        connect_control(started.metadata()).await.unwrap();
        let stopped = super::super::windows_lifecycle::shutdown_host(started.metadata())
            .await
            .unwrap();
        super::super::windows_lifecycle::await_host_stopped(&stopped)
            .await
            .unwrap();
        assert!(owner.leader_exited());
    });
}

#[test]
fn cancellation_and_timeout_terminate_the_whole_provisional_job() {
    if run_in_controlled_job("cancellation") {
        return;
    }
    runtime().block_on(async {
        for cancelled in [true, false] {
            let (root, endpoint) = fixture();
            let child = StartupChild::spawn(&command(root.path(), "descendants")).unwrap();
            let owner = FixtureOwner::new(&child);
            await_marker(&root.join("descendant-pid")).await;
            let pid: u32 = fs::read_to_string(root.join("descendant-pid"))
                .unwrap()
                .parse()
                .unwrap();
            let descendant = PinnedProcess::open_peer(pid).unwrap();
            if cancelled {
                assert!(
                    tokio::time::timeout(
                        Duration::from_millis(20),
                        wait_ready(&endpoint, child, Instant::now() + READINESS_BUDGET)
                    )
                    .await
                    .is_err()
                );
            } else {
                assert!(
                    wait_ready(&endpoint, child, Instant::now() + Duration::from_millis(40))
                        .await
                        .is_err()
                );
            }
            let deadline = Instant::now() + CLEANUP_BUDGET;
            while !owner.leader_exited() || descendant.is_alive().unwrap() {
                assert!(Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    });
}

#[test]
fn startup_does_not_release_on_early_exit_silent_or_invalid_welcome() {
    if run_in_controlled_job("invalid_readiness") {
        return;
    }
    runtime().block_on(async {
        for mode in [
            "exit",
            "silent",
            "wrong-welcome",
            "incompatible",
            "descendant-server",
        ] {
            let (root, endpoint) = fixture();
            let child = StartupChild::spawn(&command(root.path(), mode)).unwrap();
            let owner = FixtureOwner::new(&child);
            if mode != "exit" {
                await_marker(&root.join("configured-ready")).await;
                if mode == "incompatible" {
                    assert_eq!(
                        endpoint
                            .observe_ready()
                            .unwrap()
                            .unwrap()
                            .metadata()
                            .protocol,
                        crate::protocol::VERSION + 1
                    );
                }
            }
            let result = wait_ready(
                &endpoint,
                child,
                Instant::now() + Duration::from_millis(100),
            )
            .await;
            assert!(result.is_err(), "{mode}");
            let deadline = Instant::now() + CLEANUP_BUDGET;
            while !owner.leader_exited() {
                assert!(Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    });
}

#[test]
fn raced_external_winner_is_retained_and_owned_loser_is_terminated() {
    if run_in_controlled_job("external_winner") {
        return;
    }
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let winner = StartupChild::spawn(&command(root.path(), "serve")).unwrap();
        let winner_owner = FixtureOwner::new(&winner);
        let winner = wait_ready(&endpoint, winner, Instant::now() + READINESS_BUDGET)
            .await
            .unwrap();
        let loser = StartupChild::spawn(&command(root.path(), "idle")).unwrap();
        let loser_owner = FixtureOwner::new(&loser);
        let started = wait_ready(&endpoint, loser, Instant::now() + READINESS_BUDGET)
            .await
            .unwrap();
        assert_eq!(started.disposition(), StartDisposition::ExistingWinner);
        assert_eq!(started.metadata().process, winner.metadata().process);
        assert!(loser_owner.leader_exited());
        assert!(!winner_owner.leader_exited());
        connect_control(started.metadata()).await.unwrap();
    });
}

#[test]
fn occupied_preflight_never_spawns_and_winner_commit_rechecks_deadline_and_liveness() {
    if run_in_controlled_job("occupied") {
        return;
    }
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let child = StartupChild::spawn(&command(root.path(), "serve")).unwrap();
        let owner = FixtureOwner::new(&child);
        let started = wait_ready(&endpoint, child, Instant::now() + READINESS_BUDGET)
            .await
            .unwrap();
        assert!(retire_stale(&endpoint).unwrap());
        // Missing executable proves this occupied branch never builds/spawns.
        let existing =
            start_detached_host(&endpoint, HostStartup::new(root.join("does-not-exist.exe")))
                .await
                .unwrap();
        assert_eq!(existing.disposition(), StartDisposition::ExistingWinner);
        assert!(
            finish_winner(
                started.metadata.clone(),
                started.peer.clone(),
                Instant::now()
            )
            .is_err()
        );
        drop(owner);
        assert!(
            finish_winner(
                started.metadata,
                started.peer,
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
    });
}

#[test]
fn ready_absence_and_malformed_registry_are_never_treated_as_vacancy() {
    let (root, endpoint) = fixture();
    let publication = endpoint.prepare(None).unwrap().publish().unwrap();
    Directory::open_existing(&root.join("endpoint"), true)
        .unwrap()
        .remove(OsStr::new("endpoint.json"))
        .unwrap();
    assert!(
        retire_stale(&endpoint).unwrap(),
        "remaining configured row still blocks launch"
    );
    Directory::open_existing(&root.join("registry"), true)
        .unwrap()
        .atomic_write(
            OsStr::new(&format!("{}.json", publication.metadata().id)),
            b"bad",
        )
        .unwrap();
    assert!(retire_stale(&endpoint).is_err());
}

#[test]
fn provisional_launcher_inside_a_nonbreakaway_job_is_refused_without_weakening_it() {
    if run_in_controlled_job("provisional_refusal") {
        return;
    }
    runtime().block_on(async {
        let (root, _) = fixture();
        let child = StartupChild::spawn(&command(root.path(), "constrained-launcher")).unwrap();
        let owner = FixtureOwner::new(&child);
        await_marker(&root.join("breakaway-refused")).await;
        let deadline = Instant::now() + CLEANUP_BUDGET;
        while !owner.leader_exited() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(child.exit_status().unwrap().unwrap().success());
        assert!(owner.leader_exited());
    });
}

#[test]
fn released_host_can_start_another_host_that_survives_its_launcher_process() {
    if run_in_controlled_job("released_launcher") {
        return;
    }
    runtime().block_on(async {
        let (root, _) = fixture();
        // Every process in this acceptance scenario remains inside this root
        // job, including the interval before an escaped peer is acknowledged.
        // Killing any intermediate launcher cannot strand a fixture process.
        let child = StartupChild::spawn(&command(root.path(), "released-acceptance")).unwrap();
        let owner = FixtureOwner::new(&child);
        // The outer bound covers two independent five-second startup budgets
        // plus the first server's bounded shutdown, not one readiness attempt.
        await_marker_until(
            &root.join("acceptance-complete"),
            Instant::now() + Duration::from_secs(15),
        )
        .await;
        let deadline = Instant::now() + CLEANUP_BUDGET;
        while !owner.leader_exited() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(child.exit_status().unwrap().unwrap().success());
    });
}

async fn released_launcher_scenario(root: &Path) {
    let endpoint = location(root);
    let child = StartupChild::spawn(&command(root, "released-launcher")).unwrap();
    let owner = FixtureOwner::new(&child);
    let launcher = wait_ready(&endpoint, child, Instant::now() + READINESS_BUDGET)
        .await
        .unwrap();
    fs::write(root.join("launch-next"), b"launch").unwrap();
    await_marker(&root.join("next-launched")).await;
    let next = location(&root.join("next"));
    let (metadata, peer) = ready(&next, Instant::now() + READINESS_BUDGET)
        .await
        .unwrap()
        .unwrap();
    let _released = ReleasedFixture::new(&peer);
    fs::write(root.join("next-owned"), b"owned").unwrap();
    let deadline = Instant::now() + CLEANUP_BUDGET;
    while !owner.leader_exited() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(!launcher.peer().is_alive().unwrap());
    drop(owner);
    assert!(peer.is_alive().unwrap());
    connect_control(&metadata).await.unwrap();
}

#[test]
fn allowed_inner_job_does_not_escape_a_restrictive_outer_job() {
    if run_in_controlled_job("nested_policy") {
        return;
    }
    runtime().block_on(async {
        let (root, _) = fixture();
        let outer = StartupChild::spawn(&command(root.path(), "nested-policy")).unwrap();
        let owner = FixtureOwner::new(&outer);
        await_marker(&root.join("next-launched")).await;
        let next = location(&root.join("next"));
        let (_, peer) = ready(&next, Instant::now() + READINESS_BUDGET)
            .await
            .unwrap()
            .unwrap();
        assert!(
            outer
                .contains_process(peer.as_handle().as_raw_handle())
                .unwrap()
        );
        stop_provisional(&outer).await.unwrap();
        assert!(owner.leader_exited());
        // Job accounting and the launcher's signaled handle do not substitute
        // for observing this different retained descendant process object.
        let deadline = Instant::now() + CLEANUP_BUDGET;
        while peer.is_alive().unwrap() {
            assert!(Instant::now() < deadline, "owned nested peer did not exit");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!peer.is_alive().unwrap());
    });
}

#[test]
fn actual_spawn_missing_executable_preserves_typed_upgrade_diagnostic() {
    if run_in_controlled_job("missing_executable") {
        return;
    }
    let (root, _) = fixture();
    let mut command = Command::new(root.join("removed-after-preparation.exe"));
    command.current_dir(root.path());
    let error = match spawn_prepared(&command) {
        Ok(_) => panic!("missing native executable unexpectedly started"),
        Err(error) => error,
    };
    assert!(
        error
            .downcast_ref::<UnavailableStartupExecutable>()
            .is_some()
    );
}

fn write_private(path: &Path, bytes: &[u8]) {
    Directory::open_existing(path.parent().unwrap(), true)
        .unwrap()
        .atomic_write(path.file_name().unwrap(), bytes)
        .unwrap();
}

fn stale_configured_fixture() -> (
    TestRuntimeRoot,
    EndpointLocation,
    Publication,
    EndpointMetadata,
    Vec<PathBuf>,
) {
    let root = TestRuntimeRoot::new("startup-stale-records").unwrap();
    fs::create_dir(root.join("project")).unwrap();
    let roots = [
        root.join("registry-a"),
        root.join("registry-b"),
        root.join("inventory"),
    ];
    let endpoint = EndpointLocation::new(
        &root.join("project"),
        root.join("endpoint"),
        RegistrySet::with_inventory(&roots[..2], Some(roots[2].clone())).unwrap(),
    )
    .unwrap();
    let publication = endpoint.prepare(None).unwrap().publish().unwrap();
    let mut metadata = publication.metadata().clone();
    // The current PID with a different creation identity is conclusively Reused;
    // no guessed historical PID or process termination is involved.
    metadata.process.creation_time ^= 1;
    let mut paths = Vec::new();
    for registry in &roots {
        let rows = fs::read_dir(registry)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension() == Some(OsStr::new("json")))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        let path = rows.into_iter().next().unwrap();
        let mut row = RegistryRecord::from_json(&fs::read(&path).unwrap()).unwrap();
        row.host = metadata.clone();
        write_private(&path, &serde_json::to_vec(&row).unwrap());
        paths.push(path);
    }
    write_private(
        &endpoint.ready_record(),
        &serde_json::to_vec(&metadata).unwrap(),
    );
    (root, endpoint, publication, metadata, paths)
}

#[test]
fn exact_stale_configured_rows_inventory_and_ready_are_retired_before_vacancy() {
    let (_root, endpoint, _old_owner, _metadata, paths) = stale_configured_fixture();
    assert!(!retire_stale(&endpoint).unwrap());
    assert!(!endpoint.ready_record().try_exists().unwrap());
    for path in paths {
        assert!(!path.try_exists().unwrap());
    }
    drop(endpoint.prepare(None).unwrap());
}

#[test]
fn failed_launch_cleanup_filters_identity_and_continues_after_malformed_observation() {
    for malformed in [false, true] {
        let (_root, endpoint, _old_owner, metadata, paths) = stale_configured_fixture();
        let mut foreign = metadata.clone();
        foreign.process = ProcessIdentity::current().unwrap();
        let foreign_ready = serde_json::to_vec(&foreign).unwrap();
        write_private(&endpoint.ready_record(), &foreign_ready);
        let kept = if malformed {
            b"malformed foreign observation".to_vec()
        } else {
            let mut row = RegistryRecord::from_json(&fs::read(&paths[0]).unwrap()).unwrap();
            row.host = foreign;
            serde_json::to_vec(&row).unwrap()
        };
        write_private(&paths[0], &kept);
        let result = cleanup_failed_launch(&endpoint, metadata.process);
        assert_eq!(result.is_err(), malformed);
        assert_eq!(fs::read(endpoint.ready_record()).unwrap(), foreign_ready);
        assert_eq!(fs::read(&paths[0]).unwrap(), kept);
        for path in &paths[1..] {
            assert!(
                !path.try_exists().unwrap(),
                "other matching owned row must still be attempted"
            );
        }
    }
}
