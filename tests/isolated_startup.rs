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
    App::new_in_project(isolated_startup_config(Config::default()), None, &project).unwrap();
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
