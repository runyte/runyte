// SPDX-License-Identifier: MPL-2.0
#![cfg(all(unix, debug_assertions))]
use runyte::{
    test_support::{TestRuntimeRoot, isolate_user_paths},
    workspace::{lifecycle::force_shutdown_host, transport::LocalEndpoint},
    workspace_state::StateLocation,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

fn command(fixture: &TestRuntimeRoot, directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    isolate_user_paths(&mut command, fixture.path());
    command
        .args(args)
        .args([
            "--config",
            fixture.join("config/config.yaml").to_str().unwrap(),
        ])
        .env_remove(runyte::workspace::parent::ENVIRONMENT)
        .current_dir(directory)
        .stdin(Stdio::null());
    command
}
fn fixture() -> (TestRuntimeRoot, std::path::PathBuf) {
    let fixture = TestRuntimeRoot::new("home-cli").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    fixture.create_private_dir("runtime").unwrap();
    fixture.create_private_dir("config").unwrap();
    fs::write(
        fixture.join("config/config.yaml"),
        "lsp:\n  enable: false\n",
    )
    .unwrap();
    (fixture, home)
}
fn successful(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn init_uses_the_same_home_identity_and_never_leaves_a_discovery_marker() {
    let (fixture, home) = fixture();
    let location = StateLocation::out_of_tree(&home, &fixture.join("state/runyte"));
    successful(
        command(
            &fixture,
            fixture.path(),
            &["--init", home.to_str().unwrap()],
        )
        .output()
        .unwrap(),
    );
    assert!(location.as_path().is_dir());
    successful(command(&fixture, &home, &["--init", "."]).output().unwrap());
    assert_eq!(
        fs::read_dir(fixture.join("state/runyte/workspaces"))
            .unwrap()
            .count(),
        1
    );
    assert!(!home.join(".runyte").exists());
    let nested = home.join("scratch");
    fs::create_dir(&nested).unwrap();
    let output = command(&fixture, &nested, &["--mux"]).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("needs a workspace"));
    assert!(!nested.join(".runyte").exists());
    successful(
        command(&fixture, &nested, &["--init", "."])
            .output()
            .unwrap(),
    );
    assert!(nested.join(".runyte").is_dir());
    let account = fixture.create_private_dir("account").unwrap();
    let output = command(&fixture, &account, &["--init", "."])
        .env("RUNYTE_TEST_ACCOUNT_HOME", &account)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let displayed = String::from_utf8_lossy(&output.stdout);
    assert!(displayed.contains(account.to_str().unwrap()), "{displayed}");
    assert!(!displayed.contains("workspace in ~"));
    assert!(
        StateLocation::out_of_tree(&account, &fixture.join("state/runyte"))
            .as_path()
            .is_dir()
    );
    assert!(!account.join(".runyte").exists());
}

#[test]
fn bare_and_explicit_mux_home_launches_reuse_one_host_and_state_identity() {
    let (fixture, home) = fixture();
    let location = StateLocation::out_of_tree(&home, &fixture.join("state/runyte"));
    let endpoint = LocalEndpoint::discover_location_with(
        &location,
        &home,
        Some(&fixture.join("runtime")),
        fixture.path(),
    )
    .unwrap();
    struct Guard(LocalEndpoint);
    impl Drop for Guard {
        fn drop(&mut self) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let _ = rt.block_on(force_shutdown_host(&self.0));
        }
    }
    let guard = Guard(endpoint);
    fs::write(
        fixture.join("config/config.yaml"),
        "mode: mux\nlsp:\n  enable: false\n",
    )
    .unwrap();
    let mut pid = None;
    for args in [
        vec![],
        vec!["--mux"],
        vec!["-a"],
        vec!["--mux", home.to_str().unwrap()],
        vec!["-a", home.to_str().unwrap()],
    ] {
        let output = command(&fixture, &home, &args).output().unwrap();
        let published = guard
            .0
            .published_host()
            .unwrap()
            .unwrap_or_else(|| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        assert_eq!(published.project_root, home);
        assert_eq!(published.id, runyte::workspace::workspace_id(&home));
        assert_eq!(*pid.get_or_insert(published.pid), published.pid);
        assert!(!home.join(".runyte").exists());
    }
    assert_eq!(
        fs::read_dir(fixture.join("state/runyte/workspaces"))
            .unwrap()
            .count(),
        1
    );
}
