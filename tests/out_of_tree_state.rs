// SPDX-License-Identifier: MPL-2.0
#![cfg(all(unix, debug_assertions))]

use runyte::{
    test_support::{TestRuntimeRoot, isolate_user_paths, user_environment},
    workspace::{
        lifecycle::{HostStartup, restart_host, shutdown_host, start_detached_host},
        transport::LocalEndpoint,
    },
    workspace_state::StateLocation,
};
use std::{
    fs,
    process::{Command, Stdio},
};

struct HostGuard<'a>(&'a LocalEndpoint);
impl Drop for HostGuard<'_> {
    fn drop(&mut self) {
        // Best-effort cleanup after failed assertions; the fixture supervisor is
        // the independent last resort if the test process itself terminates.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = runtime.block_on(runyte::workspace::lifecycle::force_shutdown_host(self.0));
    }
}

fn fixture() -> (TestRuntimeRoot, std::path::PathBuf, StateLocation) {
    let root = TestRuntimeRoot::new("ost").unwrap();
    let project = root.create_private_dir("project").unwrap();
    fs::create_dir(project.join(".git")).unwrap();
    fs::write(project.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/config.yaml"),
        format!(
            "lsp:\n  enable: false\nworkspace:\n  state: {}\n",
            root.join("configured-absolute-state").display()
        ),
    )
    .unwrap();
    let location = StateLocation::out_of_tree(&project, &root.join("state/runyte"));
    (root, project, location)
}

#[test]
fn ide_uses_fixture_account_and_state_paths_on_every_unix_platform() {
    let (root, project, location) = fixture();
    for name in ["one.bin", "two.bin"] {
        fs::write(project.join(name), [0u8, 1, 2]).unwrap();
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    isolate_user_paths(&mut command, root.path());
    let output = command
        .args(["--ide", "one.bin", "two.bin"])
        .args([
            "--config",
            root.join("config/config.yaml").to_str().unwrap(),
        ])
        .env("RUNYTE_TEST_OUT_OF_TREE_ROOT", &project)
        .env_remove(runyte::workspace::parent::ENVIRONMENT)
        .current_dir(&project)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success()); // target admission stops before a TTY is needed
    let logs = fs::read_dir(location.as_path())
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("standalone-")
        })
        .count();
    assert_eq!(logs, 1, "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!project.join(".runyte").exists());
    assert!(!root.join("home/.runyte").exists());
}

#[test]
fn mux_detached_start_reconstruction_and_restart_keep_the_selected_state_namespace() {
    for use_runtime in [false, true] {
        let (root, project, location) = fixture();
        location
            .prepare(&[root.join("config"), root.join("cache")])
            .unwrap();
        let selected_runtime = if use_runtime {
            root.create_private_dir("runtime").unwrap()
        } else {
            root.join("absent-runtime")
        };
        let endpoint = LocalEndpoint::discover_location_with(
            &location,
            &project,
            use_runtime.then_some(selected_runtime.as_path()),
            root.path(),
        )
        .unwrap();
        let startup = || {
            HostStartup::new(env!("CARGO_BIN_EXE_runyte"), "out-of-tree fixture")
                .with_config(Some(&root.join("config/config.yaml")))
                .with_envs(user_environment(root.path()))
                .with_env("XDG_RUNTIME_DIR", &selected_runtime)
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let guard = HostGuard(&endpoint);
        runtime.block_on(async {
            start_detached_host(&endpoint, startup()).await.unwrap();
            assert!(endpoint.socket().exists());
            assert!(location.as_path().join("host.log").exists());
            let recovered = LocalEndpoint::from_parent_metadata(endpoint.metadata()).unwrap();
            assert_eq!(recovered.socket(), endpoint.socket());
            restart_host(&recovered, startup()).await.unwrap();
            assert!(endpoint.socket().exists());
            shutdown_host(&recovered).await.unwrap();
        });
        assert!(!project.join(".runyte").exists());
        assert!(!root.join("configured-absolute-state").exists());
        drop(guard);
    }
}
