// SPDX-License-Identifier: MPL-2.0

//! Plain sessions, exercised through the real launch path.
//!
//! A standalone launch that names two binary files fails while opening its
//! targets: after the workspace decision and logging, but before it needs a
//! terminal. That makes the decision observable without a PTY. A launch that
//! still resolved a workspace from a directory with none would instead stop at
//! the project-directory question, which a null stdin cannot answer.

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use runyte::test_support::TestRuntimeRoot;

/// A private directory beside the project for one kind of per-user state.
fn process_dir(owner: &Path, kind: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = owner.join(kind);
    fs::create_dir_all(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

/// Runs the bundled editor in `directory` with every per-user location
/// pointed into `owner`, so nothing reaches the person's configuration,
/// runtime registry, or cache.
fn runyte(owner: &Path, directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(arguments)
        .current_dir(directory)
        .env_remove(runyte::workspace::parent::ENVIRONMENT)
        .env("XDG_CONFIG_HOME", process_dir(owner, "config"))
        .env("XDG_RUNTIME_DIR", process_dir(owner, "run"))
        .env("XDG_CACHE_HOME", process_dir(owner, "cache"))
        .env(
            "RUNYTE_ALL_HOSTS_DIR",
            process_dir(owner, "run").join("runyte/all-hosts"),
        )
        .env("RUNYTE_TEST_SUPERVISOR_PID", std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

/// Two binary files in `directory`, the launch that fails after startup has
/// decided everything this test observes.
fn binary_targets(directory: &Path) -> [&'static str; 2] {
    fs::write(directory.join("first.bin"), [0u8, 1, 2, 3]).unwrap();
    fs::write(directory.join("second.bin"), [0u8, 4, 5, 6]).unwrap();
    ["first.bin", "second.bin"]
}

fn standalone_logs(state: &Path) -> Vec<String> {
    fs::read_dir(state)
        .map(|entries| {
            entries
                .filter_map(|entry| {
                    let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                    name.starts_with("standalone-").then_some(name)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_file_launch_outside_any_workspace_opens_without_asking_or_writing_state() {
    let owner = TestRuntimeRoot::new("plain-launch").unwrap();
    let directory = owner.join("etc");
    fs::create_dir_all(&directory).unwrap();
    let targets = binary_targets(&directory);

    let output = runyte(&owner, &directory, &targets);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the binary targets fail the launch"
    );
    assert!(
        !stderr.contains("Project directory") && !stderr.contains("not confirmed"),
        "a file launch with no workspace must not ask for one: {stderr}"
    );
    assert!(
        !directory.join(".runyte").exists(),
        "a plain session creates no state directory"
    );
}

#[test]
fn a_bare_launch_outside_any_workspace_still_asks() {
    let owner = TestRuntimeRoot::new("plain-bare-launch").unwrap();
    let directory = owner.join("empty");
    fs::create_dir_all(&directory).unwrap();

    let output = runyte(&owner, &directory, &[]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("No Git repository"), "{stderr}");
    assert!(stderr.contains("not confirmed"), "{stderr}");
    assert!(!directory.join(".runyte").exists());
}

#[test]
fn plain_keeps_a_launch_inside_a_workspace_out_of_its_state() {
    let owner = TestRuntimeRoot::new("plain-in-workspace").unwrap();
    let project = owner.join("project");
    let state = project.join(".runyte");
    fs::create_dir_all(&state).unwrap();
    let targets = binary_targets(&project);

    let plain = runyte(&owner, &project, &["--plain", targets[0], targets[1]]);
    assert!(!plain.status.success());
    assert_eq!(
        standalone_logs(&state),
        Vec::<String>::new(),
        "--plain writes no log into the workspace it was launched in"
    );

    // The same launch without --plain belongs to the workspace and logs there.
    let workspace = runyte(&owner, &project, &targets);
    assert!(!workspace.status.success());
    assert_eq!(standalone_logs(&state).len(), 1);
}

#[test]
fn plain_with_an_explicit_log_still_writes_it() {
    let owner = TestRuntimeRoot::new("plain-explicit-log").unwrap();
    let directory = owner.join("etc");
    fs::create_dir_all(&directory).unwrap();
    let targets = binary_targets(&directory);
    let log = owner.join("plain.log");

    let output = runyte(
        &owner,
        &directory,
        &["--log", log.to_str().unwrap(), targets[0], targets[1]],
    );

    assert!(!output.status.success());
    let text = fs::read_to_string(&log).unwrap();
    assert!(text.contains("ERROR"), "{text}");
    assert!(!directory.join(".runyte").exists());
}
