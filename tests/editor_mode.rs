// SPDX-License-Identifier: MPL-2.0

//! Editor mode and the workspace requirement of ide and mux mode, exercised
//! through the real launch path.
//!
//! A standalone launch that names two binary files fails while opening its
//! targets: after the workspace decision and logging, but before it needs a
//! terminal. That makes the decision observable without a PTY.

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
    run(
        Path::new(env!("CARGO_BIN_EXE_runyte")),
        owner,
        directory,
        arguments,
    )
}

fn run(program: &Path, owner: &Path, directory: &Path, arguments: &[&str]) -> Output {
    Command::new(program)
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
fn a_launch_outside_any_workspace_refuses_and_points_to_init() {
    let owner = TestRuntimeRoot::new("ide-outside-workspace").unwrap();
    let directory = owner.join("etc");
    fs::create_dir_all(&directory).unwrap();
    let targets = binary_targets(&directory);

    for arguments in [
        &[][..],
        &targets[..],
        &["--ide", targets[0]][..],
        &["--mux"][..],
    ] {
        let output = runyte(&owner, &directory, arguments);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{arguments:?}");
        assert!(
            stderr.contains("no workspace here; run runyte --init DIRECTORY to create one"),
            "{arguments:?}: {stderr}"
        );
        assert!(!stderr.contains("Project directory"), "{stderr}");
        assert!(!directory.join(".runyte").exists(), "{arguments:?}");
    }
}

#[test]
fn editor_mode_opens_outside_any_workspace_without_writing_state() {
    let owner = TestRuntimeRoot::new("editor-outside-workspace").unwrap();
    let directory = owner.join("etc");
    fs::create_dir_all(&directory).unwrap();
    let targets = binary_targets(&directory);

    let output = runyte(&owner, &directory, &["--editor", targets[0], targets[1]]);

    // The launch got as far as opening its targets, which is where two
    // binary files fail it; a refused workspace would have stopped earlier.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("binary"), "{stderr}");
    assert!(!directory.join(".runyte").exists());
}

#[test]
fn editor_mode_keeps_a_launch_inside_a_workspace_out_of_its_state() {
    let owner = TestRuntimeRoot::new("editor-in-workspace").unwrap();
    let project = owner.join("project");
    let state = project.join(".runyte");
    fs::create_dir_all(&state).unwrap();
    let targets = binary_targets(&project);

    let editor = runyte(&owner, &project, &["--editor", targets[0], targets[1]]);
    assert!(!editor.status.success());
    assert_eq!(
        standalone_logs(&state),
        Vec::<String>::new(),
        "editor mode writes no log into the workspace it was launched in"
    );

    // The same launch in ide mode belongs to the workspace and logs there.
    let workspace = runyte(&owner, &project, &targets);
    assert!(!workspace.status.success());
    assert_eq!(standalone_logs(&state).len(), 1);
}

#[test]
fn editor_mode_with_an_explicit_log_still_writes_it() {
    let owner = TestRuntimeRoot::new("editor-explicit-log").unwrap();
    let directory = owner.join("etc");
    fs::create_dir_all(&directory).unwrap();
    let targets = binary_targets(&directory);
    let log = owner.join("editor.log");

    let output = runyte(
        &owner,
        &directory,
        &[
            "--editor",
            "--log",
            log.to_str().unwrap(),
            targets[0],
            targets[1],
        ],
    );

    assert!(!output.status.success());
    let text = fs::read_to_string(&log).unwrap();
    assert!(text.contains("ERROR"), "{text}");
    assert!(!directory.join(".runyte").exists());
}

#[test]
fn init_creates_a_workspace_and_exits_without_opening_it() {
    let owner = TestRuntimeRoot::new("init-only").unwrap();
    let directory = owner.join("project");
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();

    // Null stdin and no terminal: initializing must need neither.
    let output = runyte(&owner, &owner, &["--init", directory.to_str().unwrap()]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join(".runyte").is_dir());
    assert!(
        stdout.contains(&format!("initialized a workspace in {}", root.display())),
        "{stdout}"
    );
    assert!(stdout.contains("runyte --mux"), "{stdout}");

    // The workspace now satisfies an ide launch from inside it.
    let targets = binary_targets(&root);
    let ide = runyte(&owner, &root, &targets);
    let stderr = String::from_utf8_lossy(&ide.stderr);
    assert!(!stderr.contains("no workspace here"), "{stderr}");

    let again = runyte(&owner, &owner, &["--init", directory.to_str().unwrap()]);
    assert!(again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("is already a workspace"),
        "{}",
        String::from_utf8_lossy(&again.stdout)
    );
}

#[test]
fn a_configured_editor_mode_needs_no_workspace_and_a_mode_option_overrides_it() {
    let owner = TestRuntimeRoot::new("configured-editor").unwrap();
    let directory = owner.join("etc");
    fs::create_dir_all(&directory).unwrap();
    let targets = binary_targets(&directory);
    let config = owner.join("editor.yaml");
    fs::write(&config, "mode: editor\nlsp:\n  enable: false\n").unwrap();
    let config = config.to_str().unwrap();

    let editor = runyte(
        &owner,
        &directory,
        &["--config", config, targets[0], targets[1]],
    );
    let stderr = String::from_utf8_lossy(&editor.stderr);
    assert!(stderr.contains("binary"), "{stderr}");
    assert!(!directory.join(".runyte").exists());

    let ide = runyte(
        &owner,
        &directory,
        &["--config", config, "--ide", targets[0], targets[1]],
    );
    let stderr = String::from_utf8_lossy(&ide.stderr);
    assert!(stderr.contains("no workspace here"), "{stderr}");
}

#[test]
fn runed_is_editor_mode_whatever_the_configuration_says() {
    let owner = TestRuntimeRoot::new("runed").unwrap();
    let directory = owner.join("etc");
    fs::create_dir_all(&directory).unwrap();
    let targets = binary_targets(&directory);
    // The installer's link, pointing at the built binary rather than at an
    // executable this test wrote.
    let runed = owner.join("runed");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_runyte"), &runed).unwrap();
    let config = owner.join("mux.yaml");
    fs::write(&config, "mode: mux\n").unwrap();
    let config = config.to_str().unwrap();

    let editor = run(
        &runed,
        &owner,
        &directory,
        &["--config", config, targets[0], targets[1]],
    );
    let stderr = String::from_utf8_lossy(&editor.stderr);
    assert!(stderr.contains("binary"), "{stderr}");
    assert!(!directory.join(".runyte").exists());

    let refused = run(&runed, &owner, &directory, &["--ide", targets[0]]);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success());
    assert!(
        stderr.contains("runed always runs in editor mode"),
        "{stderr}"
    );
}
