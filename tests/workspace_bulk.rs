// SPDX-License-Identifier: MPL-2.0

#![cfg(unix)]

use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn sandbox() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rwb-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            % 1_000_000_007
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn run_cli(directory: &Path, runtime: &Path, cache: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(args)
        .current_dir(directory)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("XDG_CACHE_HOME", cache)
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
    for _ in 0..200 {
        let listing = run_cli(directory, runtime, cache, &["--session-list"]);
        if listing.status.success() {
            let output = String::from_utf8(listing.stdout).unwrap();
            if needles.iter().all(|needle| output.contains(needle)) {
                return Some(output);
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    None
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

#[test]
fn stop_all_then_clear_all_manages_the_complete_workspace_inventory() {
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
    thread::sleep(Duration::from_millis(100));
    if socket_creation_is_unavailable(&mut first_host)
        || socket_creation_is_unavailable(&mut second_host)
    {
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let first_display = first.canonicalize().unwrap().display().to_string();
    let second_display = second.canonicalize().unwrap().display().to_string();
    let Some(listing) = wait_for_listing(
        &root,
        &runtime,
        &cache,
        &[&first_display, &second_display, "running"],
    ) else {
        if socket_creation_is_unavailable(&mut first_host)
            || socket_creation_is_unavailable(&mut second_host)
        {
            fs::remove_dir_all(root).unwrap();
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

    let cleared = run_cli(&root, &runtime, &cache, &["--session-clear-all"]);
    assert!(
        cleared.status.success(),
        "{}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    assert!(
        String::from_utf8(cleared.stdout)
            .unwrap()
            .contains("cleared 2 stopped sessions")
    );
    let listing = run_cli(&root, &runtime, &cache, &["--session-list"]);
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(!listing.contains(&first_display), "{listing}");
    assert!(!listing.contains(&second_display), "{listing}");

    fs::remove_dir_all(root).unwrap();
}
