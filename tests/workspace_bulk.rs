// SPDX-License-Identifier: MPL-2.0

#![cfg(unix)]

use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use runyte::{test_support::TestRuntimeRoot, workspace::transport::LocalEndpoint};

const EVENTUALLY_TIMEOUT: Duration = Duration::from_secs(10);
const EVENTUALLY_INTERVAL: Duration = Duration::from_millis(25);

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn sandbox() -> TestRuntimeRoot {
    TestRuntimeRoot::new("bulk").unwrap()
}

fn inventory_registrations(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root.join("all-hosts"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect()
}

fn sole_inventory_registration(root: &Path) -> PathBuf {
    let registrations = inventory_registrations(root);
    assert_eq!(
        registrations.len(),
        1,
        "one test host must publish exactly one owner-wide registration"
    );
    registrations.into_iter().next().unwrap()
}

fn cli_command(directory: &Path, runtime: &Path, cache: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .args(args)
        .current_dir(directory)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("XDG_CACHE_HOME", cache)
        .env(
            "RUNYTE_ALL_HOSTS_DIR",
            cache.parent().unwrap().join("all-hosts"),
        )
        .env("RUNYTE_TEST_SUPERVISOR_PID", std::process::id().to_string());
    command
}

fn run_cli(directory: &Path, runtime: &Path, cache: &Path, args: &[&str]) -> Output {
    cli_command(directory, runtime, cache, args)
        .output()
        .unwrap()
}

fn spawn_host(project: &Path, runtime: &Path, cache: &Path) -> ChildGuard {
    ChildGuard(Some(
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .arg("--serve")
            .current_dir(project)
            .env("XDG_RUNTIME_DIR", runtime)
            .env("XDG_CACHE_HOME", cache)
            .env(
                "RUNYTE_ALL_HOSTS_DIR",
                cache.parent().unwrap().join("all-hosts"),
            )
            .env("RUNYTE_TEST_SUPERVISOR_PID", std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ))
}

fn wait_for_listing(
    directory: &Path,
    runtime: &Path,
    cache: &Path,
    needles: &[&str],
) -> Option<String> {
    wait_for_listing_with_running_count(directory, runtime, cache, needles, 0)
}

fn wait_for_listing_with_running_count(
    directory: &Path,
    runtime: &Path,
    cache: &Path,
    needles: &[&str],
    running_count: usize,
) -> Option<String> {
    let deadline = Instant::now() + EVENTUALLY_TIMEOUT;
    loop {
        let listing = run_cli(directory, runtime, cache, &["--session-list"]);
        if listing.status.success() {
            let output = String::from_utf8(listing.stdout).unwrap();
            if needles.iter().all(|needle| output.contains(needle))
                && output.matches("running").count() >= running_count
            {
                return Some(output);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        thread::sleep(EVENTUALLY_INTERVAL.min(remaining));
    }
}

fn socket_creation_is_unavailable(child: &mut ChildGuard) -> bool {
    let Some(child) = child.0.as_mut() else {
        return false;
    };
    let Ok(Some(_)) = child.try_wait() else {
        return false;
    };
    let mut stderr = String::new();
    child
        .stderr
        .as_mut()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(
        stderr.contains("Operation not permitted"),
        "workspace host exited unexpectedly: {stderr}"
    );
    true
}

fn process_exists(pid: u32) -> bool {
    // SAFETY: signal zero does not deliver a signal and `pid` came from this
    // test's own freshly spawned child.
    (unsafe { libc::kill(pid as libc::pid_t, 0) }) == 0
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn process_is_running(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        // A detached child is reparented when the helper dies. Minimal test
        // init processes do not necessarily reap it promptly, so a zombie is
        // proof that the host exited even while signal-zero still finds it.
        return stat
            .rsplit_once(") ")
            .and_then(|(_, suffix)| suffix.chars().next())
            .is_some_and(|state| state != 'Z' && state != 'X');
    }
    process_exists(pid)
}

#[test]
#[ignore = "subprocess helper for detached-host supervision"]
fn detached_host_supervision_helper() {
    let Some(root) = std::env::var_os("RUNYTE_SUPERVISION_HELPER_ROOT").map(PathBuf::from) else {
        return;
    };
    let runtime = root.join("runtime");
    let cache = root.join("cache");
    let project = root.join("project");
    let launcher = cli_command(&project, &runtime, &cache, &["--persistent"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fs::write(root.join("launcher.pid"), launcher.id().to_string()).unwrap();
    let started = launcher.wait_with_output().unwrap();
    assert!(
        !started.status.success(),
        "the non-terminal attachment unexpectedly reached a TUI"
    );
    let stderr = String::from_utf8_lossy(&started.stderr);
    assert!(
        stderr.contains("raw mode")
            || stderr.contains("reader source not set")
            || stderr.contains("terminal event")
            || stderr.contains("terminal input"),
        "detached host launch failed before attachment: {stderr}"
    );
    // Host readiness is observed independently by the parent through the
    // endpoint. This marker says only that the launcher exited and the helper
    // has finished launching children and is parked for the kill.
    fs::write(root.join("helper.ready"), b"ready").unwrap();
    loop {
        thread::park();
    }
}

#[test]
fn detached_host_exits_and_unpublishes_when_its_test_runner_is_killed() {
    let root = sandbox();
    let runtime = root.join("runtime");
    let cache = root.join("cache");
    let project = root.join("project");
    fs::create_dir_all(project.join(".runyte")).unwrap();
    fs::create_dir_all(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();

    let mut helper = ChildGuard(Some(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "detached_host_supervision_helper",
                "--nocapture",
            ])
            .env("RUNYTE_SUPERVISION_HELPER_ROOT", root.as_os_str())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let project_display = project.canonicalize().unwrap().display().to_string();
    let deadline = Instant::now() + EVENTUALLY_TIMEOUT;
    let running = loop {
        let listing = run_cli(&root, &runtime, &cache, &["--session-list"]);
        let running = listing.status.success()
            && String::from_utf8_lossy(&listing.stdout).contains(&project_display)
            && String::from_utf8_lossy(&listing.stdout).contains("running");
        if running {
            break true;
        }
        if helper.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            let mut stderr = String::new();
            helper
                .0
                .as_mut()
                .unwrap()
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            if stderr.contains("Operation not permitted") {
                return;
            }
            panic!("supervision helper exited before starting its host: {stderr}");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break false;
        }
        thread::sleep(EVENTUALLY_INTERVAL.min(remaining));
    };
    if !running {
        let status = helper.0.as_mut().unwrap().try_wait().unwrap();
        let mut stderr = String::new();
        if status.is_some() {
            helper
                .0
                .as_mut()
                .unwrap()
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
        }
        if stderr.contains("Operation not permitted") {
            let _ = helper.0.take().unwrap().wait();
            return;
        }
        panic!("supervised host did not publish its endpoint: {status:?}: {stderr}");
    }

    let endpoint = LocalEndpoint::discover_with_runtime(
        &project.join(".runyte"),
        &project.canonicalize().unwrap(),
        Some(&runtime),
    )
    .unwrap();
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(endpoint.metadata()).unwrap()).unwrap();
    let endpoint_directory = endpoint.socket().parent().unwrap().to_path_buf();
    let inventory_registration = sole_inventory_registration(&root);
    let host_pid = metadata["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .expect("detached host endpoint did not contain a valid PID");

    let marker = root.join("helper.ready");
    let deadline = Instant::now() + EVENTUALLY_TIMEOUT;
    loop {
        if fs::read(&marker).is_ok_and(|value| value == b"ready") {
            break;
        }
        if helper.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            let mut stderr = String::new();
            helper
                .0
                .as_mut()
                .unwrap()
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("supervision helper exited before parking: {stderr}");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let launcher = fs::read_to_string(root.join("launcher.pid")).ok();
            panic!("supervision helper did not park after launcher {launcher:?} exited");
        }
        thread::sleep(EVENTUALLY_INTERVAL.min(remaining));
    }

    helper.0.as_mut().unwrap().kill().unwrap();
    let _ = helper.0.take().unwrap().wait();
    let deadline = Instant::now() + EVENTUALLY_TIMEOUT;
    loop {
        let listing = run_cli(
            &root,
            &runtime,
            &cache,
            &["--session-list", "--include-hidden"],
        );
        let output = String::from_utf8_lossy(&listing.stdout);
        let row_is_stopped = output
            .lines()
            .find(|line| line.contains(&project.display().to_string()))
            .is_none_or(|line| line.contains("stopped"));
        if !process_is_running(host_pid) && row_is_stopped && !inventory_registration.exists() {
            assert!(
                !endpoint_directory.exists(),
                "retired test host left its private endpoint directory"
            );
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(EVENTUALLY_INTERVAL.min(remaining));
    }

    let listing = run_cli(
        &root,
        &runtime,
        &cache,
        &["--session-list", "--include-hidden"],
    );
    let process_state = fs::read_to_string(format!("/proc/{host_pid}/stat")).ok();
    // SAFETY: the marker was written by this test's helper for its own host.
    let _ = unsafe { libc::kill(host_pid as libc::pid_t, libc::SIGKILL) };
    panic!(
        "test-scoped host survived its supervising test process; state={process_state:?}; \
         listing={:?}; stderr={:?}",
        String::from_utf8_lossy(&listing.stdout),
        String::from_utf8_lossy(&listing.stderr)
    );
}

#[test]
fn child_guard_reaps_a_test_host_during_panic_unwinding() {
    let root = sandbox();
    let runtime = root.join("runtime");
    let cache = root.join("cache");
    let project = root.join("project");
    fs::create_dir_all(project.join(".runyte")).unwrap();
    fs::create_dir_all(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let mut host = spawn_host(&project, &runtime, &cache);
    let display = project.canonicalize().unwrap().display().to_string();
    let Some(_) = wait_for_listing(&root, &runtime, &cache, &[&display, "running"]) else {
        if socket_creation_is_unavailable(&mut host) {
            return;
        }
        panic!("workspace host did not become running");
    };
    let pid = host.0.as_ref().unwrap().id();
    let inventory_registration = sole_inventory_registration(&root);

    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = host;
        panic!("exercise ChildGuard during unwinding");
    }));
    assert!(unwound.is_err());
    assert!(!process_exists(pid));

    // The abrupt kill cannot run host cleanup. Scanning the explicit inventory
    // retires only the conclusively stale row and leaves no live process.
    let listing = run_cli(
        &root,
        &runtime,
        &cache,
        &["--session-list", "--include-hidden"],
    );
    assert!(listing.status.success());
    assert!(!inventory_registration.exists());
}

#[test]
fn stop_all_then_clean_manages_the_complete_workspace_inventory() {
    let root = sandbox();
    let runtime = root.join("runtime");
    let cache = root.join("cache");
    fs::create_dir_all(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let first = root.join("first");
    let second = root.join("second");
    for project in [&first, &second] {
        fs::create_dir_all(project.join(".runyte")).unwrap();
    }

    let mut first_host = spawn_host(&first, &runtime, &cache);
    let mut second_host = spawn_host(&second, &runtime, &cache);
    if socket_creation_is_unavailable(&mut first_host)
        || socket_creation_is_unavailable(&mut second_host)
    {
        return;
    }
    let first_display = first.canonicalize().unwrap().display().to_string();
    let second_display = second.canonicalize().unwrap().display().to_string();
    let Some(listing) = wait_for_listing_with_running_count(
        &root,
        &runtime,
        &cache,
        &[&first_display, &second_display, "running"],
        2,
    ) else {
        if socket_creation_is_unavailable(&mut first_host)
            || socket_creation_is_unavailable(&mut second_host)
        {
            return;
        }
        panic!("workspace hosts did not become running");
    };
    assert_eq!(listing.matches("running").count(), 2, "{listing}");

    let stopped = run_cli(&root, &runtime, &cache, &["--session-stop-all"]);
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(
        String::from_utf8(stopped.stdout)
            .unwrap()
            .contains("stopped 2 sessions")
    );
    assert!(first_host.0.take().unwrap().wait().unwrap().success());
    assert!(second_host.0.take().unwrap().wait().unwrap().success());

    let listing = wait_for_listing(
        &root,
        &runtime,
        &cache,
        &[&first_display, &second_display, "stopped"],
    )
    .expect("stopped sessions remained in recent history");
    assert_eq!(listing.matches("stopped").count(), 2, "{listing}");

    let cleared = run_cli(&root, &runtime, &cache, &["--session-clean"]);
    assert!(
        cleared.status.success(),
        "{}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    assert!(
        String::from_utf8(cleared.stdout)
            .unwrap()
            .contains("forgot 2 stopped sessions")
    );
    let listing = run_cli(&root, &runtime, &cache, &["--session-list"]);
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(!listing.contains(&first_display), "{listing}");
    assert!(!listing.contains(&second_display), "{listing}");
}

#[test]
fn include_hidden_lists_and_stops_hosts_outside_the_current_environment() {
    let root = sandbox();
    let first_runtime = root.join("runtime-one");
    let second_runtime = root.join("runtime-two");
    let first_cache = root.join("cache-one");
    let second_cache = root.join("cache-two");
    for runtime in [&first_runtime, &second_runtime] {
        fs::create_dir_all(runtime).unwrap();
        fs::set_permissions(runtime, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let first = root.join("first");
    let second = root.join("second");
    for project in [&first, &second] {
        fs::create_dir_all(project.join(".runyte")).unwrap();
    }

    let mut first_host = spawn_host(&first, &first_runtime, &first_cache);
    let mut second_host = spawn_host(&second, &second_runtime, &second_cache);
    let first_display = first.canonicalize().unwrap().display().to_string();
    let second_display = second.canonicalize().unwrap().display().to_string();
    let Some(_) = wait_for_listing(
        &root,
        &first_runtime,
        &first_cache,
        &[&first_display, "running"],
    ) else {
        if socket_creation_is_unavailable(&mut first_host)
            || socket_creation_is_unavailable(&mut second_host)
        {
            return;
        }
        panic!("workspace hosts did not become running");
    };
    let Some(_) = wait_for_listing(
        &root,
        &second_runtime,
        &second_cache,
        &[&second_display, "running"],
    ) else {
        if socket_creation_is_unavailable(&mut first_host)
            || socket_creation_is_unavailable(&mut second_host)
        {
            return;
        }
        panic!("second workspace host did not become running");
    };

    let local = run_cli(&root, &first_runtime, &first_cache, &["--session-list"]);
    let local = String::from_utf8(local.stdout).unwrap();
    assert!(local.contains(&first_display), "{local}");
    assert!(!local.contains(&second_display), "{local}");

    let all = run_cli(
        &root,
        &first_runtime,
        &first_cache,
        &["--session-list", "--include-hidden"],
    );
    assert!(
        all.status.success(),
        "{}",
        String::from_utf8_lossy(&all.stderr)
    );
    let all = String::from_utf8(all.stdout).unwrap();
    assert!(all.contains(&first_display), "{all}");
    assert!(all.contains(&second_display), "{all}");

    let stopped_local = run_cli(&root, &first_runtime, &first_cache, &["--session-stop-all"]);
    assert!(
        stopped_local.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped_local.stderr)
    );
    assert!(first_host.0.take().unwrap().wait().unwrap().success());
    assert!(
        second_host
            .0
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none(),
        "current-namespace stop reached the other namespace"
    );

    let stopped = run_cli(
        &root,
        &first_runtime,
        &first_cache,
        &["--session-stop-all", "--include-hidden"],
    );
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(second_host.0.take().unwrap().wait().unwrap().success());
}

#[test]
fn identical_workspace_hosts_remain_isolated_until_an_explicit_owner_wide_stop() {
    let root = sandbox();
    let first_runtime = root.join("runtime-one");
    let second_runtime = root.join("runtime-two");
    let first_cache = root.join("cache-one");
    let second_cache = root.join("cache-two");
    for runtime in [&first_runtime, &second_runtime] {
        fs::create_dir_all(runtime).unwrap();
        fs::set_permissions(runtime, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let project = root.join("project");
    fs::create_dir_all(project.join(".runyte")).unwrap();
    let display = project.canonicalize().unwrap().display().to_string();

    let mut first_host = spawn_host(&project, &first_runtime, &first_cache);
    let mut second_host = spawn_host(&project, &second_runtime, &second_cache);
    for (runtime, cache, host) in [
        (&first_runtime, &first_cache, &mut first_host),
        (&second_runtime, &second_cache, &mut second_host),
    ] {
        if wait_for_listing(&root, runtime, cache, &[&display, "running"]).is_none() {
            if socket_creation_is_unavailable(host) {
                return;
            }
            panic!("same-workspace host did not become running");
        }
    }

    let inventory_rows = fs::read_dir(root.join("all-hosts"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    assert_eq!(inventory_rows.len(), 2);
    assert_ne!(inventory_rows[0].file_name(), inventory_rows[1].file_name());

    let listed = run_cli(
        &root,
        &first_runtime,
        &first_cache,
        &["--session-list", "--include-hidden"],
    );
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert_eq!(
        String::from_utf8(listed.stdout)
            .unwrap()
            .lines()
            .filter(|line| line.contains(&display) && line.contains("running"))
            .count(),
        2
    );

    let stopped = run_cli(
        &root,
        &first_runtime,
        &first_cache,
        &["--session-stop-all", "--include-hidden"],
    );
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(first_host.0.take().unwrap().wait().unwrap().success());
    assert!(second_host.0.take().unwrap().wait().unwrap().success());
}
