// SPDX-License-Identifier: MPL-2.0

use std::{collections::HashSet, fs, path::Path, process::Command};

fn repository_files_below(root: &Path, directory: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            repository_files_below(root, &path, files);
        } else {
            files.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

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
fn published_crate_contains_the_runtime_inputs_and_not_repository_context() {
    let output = Command::new(env!("CARGO"))
        .args(["package", "--list", "--locked", "--allow-dirty"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = String::from_utf8(output.stdout).unwrap();
    let files = files.lines().collect::<HashSet<_>>();
    for required in [
        "Cargo.lock",
        "Cargo.toml",
        "LICENSE",
        "NOTICE",
        "README.md",
        "THIRD_PARTY_NOTICES.md",
        "config.example.yaml",
        "logo/ascii/logo.txt",
    ] {
        assert!(files.contains(required), "crate omits {required}");
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut runtime_sources = Vec::new();
    repository_files_below(root, &root.join("src"), &mut runtime_sources);
    runtime_sources.retain(|path| !path.starts_with("src/app/tests/"));
    repository_files_below(root, &root.join("docs"), &mut runtime_sources);
    repository_files_below(root, &root.join("licenses"), &mut runtime_sources);
    for required in runtime_sources {
        assert!(files.contains(required.as_str()), "crate omits {required}");
    }
    assert!(
        files.iter().all(|path| !path.starts_with("context/")),
        "development context leaked into the crate: {files:?}"
    );
    assert!(
        files.iter().all(|path| !path.starts_with("tests/")),
        "repository-only tests leaked into the crate: {files:?}"
    );
    assert!(
        files.iter().all(|path| !path.starts_with("src/app/tests/")),
        "in-source repository tests leaked into the crate: {files:?}"
    );
}

#[test]
fn ci_enforces_the_committed_dependency_graph() {
    let workflow =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml"))
            .unwrap();
    let workflow: serde_yaml::Value = serde_yaml::from_str(&workflow).unwrap();
    let jobs = workflow["jobs"].as_mapping().unwrap();
    for (job, command) in [
        (
            "gates",
            "cargo clippy --all-targets --locked -- -D warnings",
        ),
        ("gates", "cargo test --locked"),
        ("macos", "cargo test --locked"),
        (
            "performance",
            "cargo test --release --locked --test performance -- --ignored --test-threads=1",
        ),
        ("floor", "cargo build --release --locked"),
        ("msrv", "cargo +1.88 check --all-targets --locked"),
    ] {
        let steps = jobs[serde_yaml::Value::from(job)]["steps"]
            .as_sequence()
            .unwrap();
        assert!(
            steps
                .iter()
                .any(|step| step["run"].as_str() == Some(command)),
            "CI job {job} omits locked command: {command}"
        );
    }
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
    // `host` and `client` are internal roles, not the vocabulary a reader is
    // given. The one exception is `host.log`: that is the actual file name a
    // persistent session's diagnostic log has on disk, and help has to name a
    // path somebody can open.
    let prose = help.replace("host.log", "");
    assert!(!prose.contains("host"));
    assert!(!prose.contains("client"));
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
