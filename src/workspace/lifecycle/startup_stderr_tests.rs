// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::time::Instant;

const FIXTURE_ROOT: &str = "RUNYTE_STARTUP_STDERR_FIXTURE_ROOT";
const FIXTURE_CASE: &str = "RUNYTE_STARTUP_STDERR_FIXTURE_CASE";

fn run_fixture(case: &str) {
    let root = TestRuntimeRoot::new("startup-stderr").unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "workspace::lifecycle::startup_stderr_tests::startup_stderr_fixture",
            "--nocapture",
        ])
        .env(FIXTURE_ROOT, root.path())
        .env(FIXTURE_CASE, case)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Release a descendant even if the old blocking stderr read stranded the
    // fixture. No signal addresses a PID after its direct parent was reaped.
    fs::write(root.join("release"), []).unwrap();
    if case == "descendant" {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !root.join("descendant-done").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("startup stderr fixture {case} exceeded its bounded deadline");
    }
    assert!(
        status.unwrap().success(),
        "startup stderr fixture {case} failed"
    );
}

#[test]
fn startup_failure_does_not_wait_for_descendant_stderr_eof() {
    run_fixture("descendant");
}

#[test]
fn startup_drains_large_diagnostics_with_bounded_retention() {
    run_fixture("flood");
}

#[test]
fn detached_child_keeps_draining_diagnostics_after_readiness_owner_returns() {
    run_fixture("detached");
}

#[tokio::test]
#[ignore = "owned by the bounded startup stderr fixtures"]
async fn startup_stderr_fixture() {
    let root = PathBuf::from(std::env::var_os(FIXTURE_ROOT).unwrap());
    let case = std::env::var(FIXTURE_CASE).unwrap();
    let executable = root.join("host");
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &executable,
    )
    .unwrap();
    let behavior = match case.as_str() {
        "descendant" => {
            "printf 'startup detail retained\\n' >&2\n(while [ ! -f \"$RUNYTE_STDERR_ROOT/release\" ]; do sleep 0.01; done; printf done > \"$RUNYTE_STDERR_ROOT/descendant-done\") &\nexit 7\n"
        }
        "flood" | "detached" => {
            "head -c 1048576 /dev/zero | tr '\\000' x >&2\nprintf done > \"$RUNYTE_STDERR_ROOT/flood-done\"\nexit 7\n"
        }
        _ => panic!("unknown startup stderr fixture"),
    };
    fs::write(executable.with_extension("behavior"), behavior).unwrap();
    if case == "detached" {
        // This is the same ownership handoff made after a successful ready
        // handshake. The reaper must retain the drain, not just the Child.
        let child = Command::new(&executable)
            .env("RUNYTE_STDERR_ROOT", &root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(ReapedChild::new(child, ChildReaper::new().unwrap()).unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !root.join("flood-done").exists() {
            assert!(Instant::now() < deadline, "detached stderr writer blocked");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        return;
    }
    let runtime = root.join("runtime");
    fs::create_dir_all(&runtime).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let endpoint =
        LocalEndpoint::discover_with_runtime(&root.join(".runyte"), &root, Some(&runtime)).unwrap();
    let startup = HostStartup::new(executable, "fixture")
        .with_env("XDG_CONFIG_HOME", root.join("config"))
        .with_env("XDG_CACHE_HOME", root.join("cache"))
        .with_env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .with_env("RUNYTE_STDERR_ROOT", &root);
    let started = Instant::now();
    let error = start_detached_host(&endpoint, startup).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(3), "{error:#}");
    let message = error.to_string();
    assert!(message.contains("exited with exit status: 7"), "{message}");
    match case.as_str() {
        "descendant" => assert!(message.contains("startup detail retained"), "{message}"),
        "flood" => {
            assert!(root.join("flood-done").exists(), "stderr writer blocked");
            assert!(message.len() < 64 * 1024, "unbounded diagnostic retention");
            assert!(message.contains("startup stderr truncated"));
        }
        _ => unreachable!(),
    }
}
