// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::Path, process::Command};

#[test]
fn cargo_metadata_declares_the_editor_binary() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "runyte")
        .unwrap();
    assert_eq!(package["default_run"], "runyte");
    let mut binaries = package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|target| {
            target["kind"]
                .as_array()
                .unwrap()
                .iter()
                .any(|kind| kind == "bin")
        })
        .map(|target| target["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    binaries.sort_unstable();
    assert_eq!(binaries, ["runyte"]);
}

#[test]
fn editor_help_hides_internal_options_and_uses_workspace_modes() {
    // `--cwd-file` is an internal detail of the `runyte()` shell function
    // documented in README.md, not something anyone should pass by hand, so
    // it must not appear in the discoverable `OPTIONS:` list. `--help` must
    // still say that :quit-here exists and how to enable it, just without
    // naming the flag.
    let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("-i, --init DIRECTORY"));
    assert!(!help.contains("--cwd-file"));
    assert!(!help.contains("--project-root"));
    assert!(!help.contains("--attach"));
    for spelling in [
        "--list-workspaces",
        "--shutdown-workspace",
        "--restart-workspace",
        "--name-workspace",
        "--rename-workspace",
        "--list-hosts",
        "--shutdown-host",
        "--restart-host",
        "--name-host",
        "--rename-host",
        "--workspace-list",
        "--wls",
        "--workspace-stop",
        "--wst",
        "--workspace-stop-all",
        "--workspace-clear-all",
        "--workspace-restart",
        "--workspace-name",
        "--workspace-rename",
    ] {
        assert!(!help.contains(spelling), "help still contains {spelling}");
    }
    assert!(help.contains("--standalone"));
    assert!(help.contains("--persistent"));
    assert!(help.contains("-l, --session-list"));
    assert!(help.contains("--session-start [WORKSPACE]"));
    assert!(help.contains("-s, --session-stop [WORKSPACE]"));
    assert!(help.contains("--session-rename WORKSPACE NAME"));
    assert!(help.contains("-f, --force"));
    assert!(!help.contains("host"));
    assert!(!help.contains("client"));
    assert!(help.contains(":quit-here"));
    assert!(help.contains("runyte()"));
    assert!(help.contains("README.md"));
}

#[test]
fn cwd_file_option_still_works_though_undocumented() {
    // Hiding --cwd-file from --help must not stop it from working: the
    // documented runyte() shell function still passes it on every invocation.
    // --session-list accepts and ignores the option (see the comment above
    // its handling in src/main.rs), so it exercises the flag end to end
    // without needing a running host.
    //
    // Isolate the process from the real XDG runtime/cache directories, as
    // tests/persistent_host.rs does, so this never touches the person's own
    // workspace registry.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "runyte-cwd-file-help-test-{}-{unique}",
        std::process::id()
    ));
    let runtime_dir = temp_dir.join("runtime");
    let cache_dir = temp_dir.join("cache");
    let cwd_file = temp_dir.join("cwd");
    fs::create_dir_all(&runtime_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--cwd-file", cwd_file.to_str().unwrap(), "--session-list"])
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .env("XDG_CACHE_HOME", &cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // --session-list never writes to the handoff file.
    assert!(!cwd_file.exists() || fs::read(&cwd_file).unwrap().is_empty());

    fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn documented_shell_wrapper_avoids_zsh_read_only_parameters() {
    let readme =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    assert!(readme.contains("local runyte_tmp runyte_cwd runyte_exit"));
    assert!(!readme.contains("local tmp cwd status"));
    assert!(!readme.contains("status=$?"));
}
