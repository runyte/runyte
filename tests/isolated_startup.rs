// SPDX-License-Identifier: MPL-2.0

use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

use runyte::{
    app::App,
    config::Config,
    headless::HeadlessEditor,
    test_support::{TestRuntimeRoot, isolated_startup_config, user_environment},
};

#[test]
#[ignore = "re-executed with fixture-owned user paths by the parent test"]
fn isolated_library_constructor_helper() {
    let project = PathBuf::from(std::env::var_os("RUNYTE_TEST_ISOLATION_PROJECT").unwrap());
    // This intentionally uninjected admission proves the environment sentinel
    // reaches the normally compiled library. All selectors belong to the fixture.
    let error = App::new_in_project(Config::default(), None, &project)
        .err()
        .unwrap();
    assert!(error.to_string().contains("overlaps per-user"), "{error:#}");

    HeadlessEditor::new_in(&project).unwrap();
    HeadlessEditor::new_deferred_in(&project).unwrap();
    let config = isolated_startup_config(Config::default());
    let cloned = config.clone();
    App::new_in_project(config, None, &project).unwrap();
    // Closing the first admission must retain its explicit fixture source
    // when this clone captures fresh canonical reservations.
    App::new_in_project(cloned, None, &project).unwrap();
    assert!(!project.join(".runyte").exists());
}

#[test]
fn normally_compiled_isolated_constructors_do_not_resolve_environment_user_roots() {
    let fixture = TestRuntimeRoot::new("isolated-app").unwrap();
    let project = fixture.create_private_dir("runyte").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "isolated_library_constructor_helper",
            "--ignored",
        ])
        .envs(user_environment(fixture.path()))
        // Pair selectors consistently, while making the selected user state
        // contain the project state. An accidental real-user fallback refuses it.
        .env("XDG_STATE_HOME", fixture.path())
        .env("RUNYTE_TEST_STATE_HOME", &project)
        .env("RUNYTE_TEST_ISOLATION_PROJECT", &project)
        .env_remove("RUNYTE_TEST_OUT_OF_TREE_ROOT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!project.join(".runyte").exists());
}

#[test]
fn reserved_config_state_refuses_before_editor_construction() {
    let fixture = TestRuntimeRoot::new("refused-state").unwrap();
    let project = fixture.create_private_dir("project").unwrap();
    let config_root = fixture.create_private_dir("config").unwrap();
    std::fs::create_dir(project.join(".git")).unwrap();
    std::fs::write(project.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let note = project.join("notes.txt");
    std::fs::write(&note, "unchanged\n").unwrap();
    let rejected = config_root.join("state");
    let config = config_root.join("config.yaml");
    let source = format!(
        "mode: ide\nlsp:\n  enable: false\nworkspace:\n  state: {}\n",
        serde_json::to_string(&rejected).unwrap()
    );
    std::fs::write(&config, &source).unwrap();
    // This is the native state-refusal fixture's configuration. Without a
    // display, the terminal edition reaches the same pre-App storage admission.
    let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--ide", "--config"])
        .arg(&config)
        .arg(&note)
        .envs(user_environment(fixture.path()))
        .env_remove(runyte::workspace::parent::ENVIRONMENT)
        .env("RUNYTE_INPUT_TRACE", fixture.join("input.log"))
        .current_dir(&project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("overlaps per-user Runyte storage")
            && diagnostic.contains("cannot store project data"),
        "{diagnostic}"
    );
    assert!(!fixture.join("input.log").exists());
    assert!(!fixture.join("context").exists());
    assert!(!project.join(".runyte").exists());
    assert!(!rejected.exists());
    assert_eq!(std::fs::read_to_string(config).unwrap(), source);
    assert_eq!(std::fs::read_to_string(note).unwrap(), "unchanged\n");
}
