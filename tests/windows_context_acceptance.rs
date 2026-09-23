// SPDX-License-Identifier: MPL-2.0

#![cfg(windows)]

#[cfg(debug_assertions)]
use runyte::workspace::{
    context::storage::Registration,
    windows_process_identity::{PinnedProcess, ProcessIdentity},
};
use runyte::{
    terminal::{
        emulator::Emulator,
        pty::{Pty, PtyEvent},
    },
    test_support::TestRuntimeRoot,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const FIXTURE: &str = "public_windows_context_fixture";
#[cfg(debug_assertions)]
const GRANT_FIXTURE: &str = "public_windows_context_grant_seed_fixture";
#[cfg(debug_assertions)]
const FAILURE_FIXTURE: &str = "public_windows_context_post_service_failure_fixture";
const ROOT: &str = "RUNYTE_CONTEXT_PUBLIC_ACCEPTANCE_ROOT";
const PYTHON: &str = "RUNYTE_CONTEXT_TEST_PYTHON";
const PHASE_READY: &str = "RUNYTE_CONTEXT_PUBLIC_PHASE_READY";
const PHASE_CONTINUE: &str = "RUNYTE_CONTEXT_PUBLIC_PHASE_CONTINUE";
const RESTART_READY: &str = "RUNYTE_CONTEXT_PUBLIC_RESTART_READY";
const REVOKE_CONTINUE: &str = "RUNYTE_CONTEXT_PUBLIC_REVOKE_CONTINUE";
#[cfg(debug_assertions)]
const FAILURE_BARRIER: &str = "RUNYTE_TEST_POST_SERVICE_FAILURE_BARRIER";

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires RUNYTE_CONTEXT_TEST_PYTHON and runs in native Windows acceptance"]
fn public_windows_context_round_trip() {
    let python = std::env::var_os(PYTHON).expect("RUNYTE_CONTEXT_TEST_PYTHON is required");
    let root = TestRuntimeRoot::new("public-windows-context").unwrap();
    let context = root.create_private_dir("context").unwrap();
    let config = root.create_private_dir("config").unwrap();
    let output = std::fs::File::create(root.join("acceptance-output")).unwrap();
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env(ROOT, root.path())
            .env(PYTHON, python)
            .env("RUNYTE_CONTEXT_HOME", context)
            .env("XDG_CONFIG_HOME", config)
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            panic!(
                "public Windows context acceptance timed out:\n{}",
                std::fs::read_to_string(root.join("acceptance-output")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(root.join("acceptance-output")).unwrap()
    );
}

struct Console {
    child: Pty,
    events: mpsc::Receiver<PtyEvent>,
    screen: Emulator,
    output: Vec<u8>,
}

impl Console {
    fn spawn(binary: &Path, project: &Path, config: &Path, file: &Path) -> Self {
        let (sender, events) = mpsc::channel();
        let arguments = vec![
            "--standalone".into(),
            "--project-root".into(),
            project.to_string_lossy().into_owned(),
            "--config".into(),
            config.to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
        ];
        let child = Pty::spawn(
            binary.as_os_str(),
            &arguments,
            project,
            120,
            30,
            move |event| {
                let _ = sender.send(event);
            },
        )
        .unwrap();
        Self {
            child,
            events,
            screen: Emulator::new(120, 30),
            output: Vec::new(),
        }
    }

    fn view(&self) -> String {
        (0..self.screen.rows())
            .filter_map(|row| self.screen.grid().line(row))
            .map(|line| {
                line.iter()
                    .filter(|cell| cell.width != 0)
                    .map(|cell| cell.text())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn until(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.view().contains(needle) {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => {
                    self.screen.feed(&bytes);
                    self.output.extend(bytes);
                }
                event => panic!(
                    "waiting for {needle:?}, got {event:?}: {}",
                    String::from_utf8_lossy(&self.output)
                ),
            }
        }
    }

    fn send(&self, input: &str) {
        assert!(self.child.write(input.as_bytes().to_vec()));
    }

    fn next_output(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        match self
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(PtyEvent::Output(bytes)) => {
                self.screen.feed(&bytes);
                self.output.extend(bytes);
            }
            event => panic!(
                "waiting for a fresh terminal frame, got {event:?}: {}",
                String::from_utf8_lossy(&self.output)
            ),
        }
    }

    fn exit(&mut self) {
        assert_eq!(
            self.exit_code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&self.output)
        );
    }

    fn exit_code(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => self.output.extend(bytes),
                Ok(PtyEvent::Exited(code)) => return code,
                event => panic!("{event:?}: {}", String::from_utf8_lossy(&self.output)),
            }
        }
    }
}

#[cfg(debug_assertions)]
fn fixture_command(root: &TestRuntimeRoot, name: &str, output: &Path) -> OwnedChild {
    let stdout = std::fs::File::create(output).unwrap();
    OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--ignored", "--nocapture"])
            .env(ROOT, root.path())
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().unwrap())
            .stderr(stdout)
            .spawn()
            .unwrap(),
    )
}

#[cfg(debug_assertions)]
fn await_child(
    child: &mut OwnedChild,
    output: &Path,
    timeout: Duration,
) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "fixture timed out:\n{}",
            std::fs::read_to_string(output).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn prepare_editor_fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".runyte")).unwrap();
    let file = project.join("unicode note.txt");
    if !file.exists() {
        std::fs::write(&file, "a\u{00e7}\u{754c}\u{1f642}z").unwrap();
    }
    let config = root.join("config/config.yaml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "lsp:\n  enable: false\n").unwrap();
    (project, file, config)
}

fn assert_isolated_environment(root: &Path) {
    assert!(root.is_absolute());
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME"),
        Some(root.join("context").into_os_string())
    );
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME"),
        Some(root.join("config").into_os_string())
    );
}

fn inventory(binary: &Path, context: &Path, config: &Path) -> Value {
    let output = Command::new(binary)
        .args(["--context-list", "--json"])
        .env("RUNYTE_CONTEXT_HOME", context)
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_CACHE_HOME", config.parent().unwrap().join("cache"))
        .env("XDG_RUNTIME_DIR", config.parent().unwrap().join("runtime"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "context inventory failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn grant_remembered_edit(editor: &mut Console) {
    editor.until("Agent context access");
    editor.until("Grant access");
    let first_page = editor.view();
    if let Some(page) = first_page
        .lines()
        .find_map(|line| line.split("Page 1/").nth(1))
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|pages| pages.parse::<usize>().ok())
    {
        for current in 2..=page {
            editor.send("j");
            editor.until(&format!("Page {current}/{page}"));
        }
    }
    editor.send("3");
    editor.until("3 [x] buffer_edit");
    editor.send("r");
    editor.until("r [x] Remember; x Revoke");
    editor.send("\t");
    editor.next_output();
    editor.send("\r");
}

fn wait_inventory(binary: &Path, context: &Path, config: &Path, count: usize) -> Value {
    wait_inventory_with_output(binary, context, config, count, None)
}

fn context_diagnostics(context: &Path) -> String {
    let mut names = std::fs::read_dir(context)
        .map(|entries| {
            entries
                .take(64)
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|error| vec![format!("<read directory failed: {error}>")]);
    names.sort();
    let registrations = names
        .iter()
        .filter(|name| name.starts_with("host-") && name.ends_with(".json"))
        .map(|name| {
            let bytes = std::fs::read(context.join(name)).unwrap_or_default();
            let end = bytes.len().min(4096);
            format!("{name}: {}", String::from_utf8_lossy(&bytes[..end]))
        })
        .collect::<Vec<_>>();
    format!("context entries: {names:?}; registrations: {registrations:?}")
}

fn bounded_output(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let start = bytes.len().saturating_sub(8192);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[cfg(debug_assertions)]
fn context_record_names(context: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut records = std::fs::read_dir(context)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".json"))
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    records.sort();
    records
}

fn wait_inventory_with_output(
    binary: &Path,
    context: &Path,
    config: &Path,
    count: usize,
    output: Option<&Path>,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let value = inventory(binary, context, config);
        if value["workspaces"].as_array().unwrap().len() == count {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "unexpected inventory: {value}; {}; fixture output:\n{}",
            context_diagnostics(context),
            output.map(bounded_output).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_file(path: &Path, child: &mut OwnedChild, output: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "Python bridge exited before {}:\n{}",
            path.display(),
            std::fs::read_to_string(output).unwrap()
        );
        assert!(Instant::now() < deadline, "waiting for {}", path.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_python(
    root: &Path,
    binary: &Path,
    context: &Path,
    config: &Path,
    python: &Path,
) -> (OwnedChild, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("bridges/runyte-context");
    let output = root.join("python-output");
    let stdout = std::fs::File::create(&output).unwrap();
    let phase_ready = root.join("phase-ready");
    let phase_continue = root.join("phase-continue");
    let restart_ready = root.join("restart-ready");
    let revoke_continue = root.join("revoke-continue");
    let child = OwnedChild(
        Command::new(python)
            .args([
                "-m",
                "unittest",
                "discover",
                "-s",
                "tests",
                "-p",
                "test_windows_native.py",
                "-v",
            ])
            .current_dir(&package)
            .env("PYTHONPATH", &package)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONNOUSERSITE", "1")
            .env("RUNYTE_CONTEXT_HOME", context)
            .env("XDG_CONFIG_HOME", config)
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("RUNYTE_CONTEXT_PUBLIC_BINARY", binary)
            .env("RUNYTE_CONTEXT_PUBLIC_ROOT", context)
            .env(PHASE_READY, &phase_ready)
            .env(PHASE_CONTINUE, &phase_continue)
            .env(RESTART_READY, &restart_ready)
            .env(REVOKE_CONTINUE, &revoke_continue)
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().unwrap())
            .stderr(stdout)
            .spawn()
            .unwrap(),
    );
    (
        child,
        output,
        phase_ready,
        phase_continue,
        restart_ready,
        revoke_continue,
    )
}

#[test]
#[ignore = "reexecuted with fixture-owned storage and explicit Python"]
fn public_windows_context_fixture() {
    let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
    let context = root.join("context");
    let config_root = root.join("config");
    assert_isolated_environment(&root);
    std::fs::create_dir_all(&context).unwrap();
    std::fs::create_dir_all(&config_root).unwrap();
    let (project, file, config) = prepare_editor_fixture(&root);
    let binary = Path::new(env!("CARGO_BIN_EXE_runyte"));

    let mut first = Console::spawn(binary, &project, &config, &file);
    first.until("a\u{00e7}\u{754c}\u{1f642}z");
    first.until("NOR");
    first.send(":context-access agent\r");
    grant_remembered_edit(&mut first);
    let published = wait_inventory(binary, &context, &config_root, 1);
    assert_eq!(
        published["workspaces"][0]["root"].as_str(),
        project.to_str()
    );

    let python = PathBuf::from(std::env::var_os(PYTHON).unwrap());
    let (mut bridge, python_output, phase_ready, phase_continue, restart_ready, revoke_continue) =
        spawn_python(&root, binary, &context, &config_root, &python);
    wait_file(&phase_ready, &mut bridge, &python_output);

    first.send(":quit!\r");
    first.exit();
    drop(first);
    wait_inventory(binary, &context, &config_root, 0);

    let mut second = Console::spawn(binary, &project, &config, &file);
    second.until("a\u{00e7}\u{754c}\u{1f642}z");
    second.until("NOR");
    wait_inventory(binary, &context, &config_root, 1);
    std::fs::write(&phase_continue, b"continue").unwrap();
    wait_file(&restart_ready, &mut bridge, &python_output);

    second.send(":context-access agent\r");
    second.until("Agent context access");
    second.send("x");
    wait_inventory(binary, &context, &config_root, 0);
    std::fs::write(&revoke_continue, b"continue").unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = bridge.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "Python bridge timed out:\n{}",
            std::fs::read_to_string(&python_output).unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let python_output = std::fs::read_to_string(&python_output).unwrap();
    assert!(status.success(), "{python_output}");
    assert!(
        python_output.contains(
            "test_public_executable_discovery_edit_reconnect_remember_restart_and_revoke"
        ) && python_output.contains("OK"),
        "required public bridge acceptance did not run:\n{python_output}"
    );

    second.send(":quit\r");
    second.exit();
    drop(second);
    wait_inventory(binary, &context, &config_root, 0);
}

#[cfg(debug_assertions)]
#[test]
fn post_service_frontend_failure_joins_context_and_retires_publication() {
    let root = TestRuntimeRoot::new("public-windows-context-failure").unwrap();
    root.create_private_dir("context").unwrap();
    root.create_private_dir("config").unwrap();
    let config = root.join("config");
    let context = root.join("context");

    let seed_output = root.join("grant-seed-output");
    let mut seed = fixture_command(&root, GRANT_FIXTURE, &seed_output);
    let status = await_child(&mut seed, &seed_output, Duration::from_secs(45));
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(&seed_output).unwrap()
    );
    assert!(
        bounded_output(&seed_output)
            .contains("test public_windows_context_grant_seed_fixture ... ok"),
        "grant seed fixture did not run:\n{}",
        bounded_output(&seed_output)
    );
    assert_eq!(
        context_record_names(&context, "grant-").len(),
        1,
        "physical remembered grant was not durable; {}; seed output:\n{}",
        context_diagnostics(&context),
        bounded_output(&seed_output)
    );
    assert!(context_record_names(&context, "host-").is_empty());

    let barrier = root.join("post-service-failure");
    let failure_output = root.join("post-service-failure-output");
    let stdout = std::fs::File::create(&failure_output).unwrap();
    let mut failure = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FAILURE_FIXTURE, "--ignored", "--nocapture"])
            .env(ROOT, root.path())
            .env("RUNYTE_CONTEXT_HOME", &context)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env(FAILURE_BARRIER, &barrier)
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().unwrap())
            .stderr(stdout)
            .spawn()
            .unwrap(),
    );
    let ready = barrier.with_extension("ready");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(
            failure.0.try_wait().unwrap().is_none(),
            "failure fixture exited before the barrier:\n{}",
            std::fs::read_to_string(&failure_output).unwrap()
        );
        assert!(
            Instant::now() < deadline,
            "post-service barrier did not open"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let publication_deadline = Instant::now() + Duration::from_secs(20);
    let publication = loop {
        let publications = context_record_names(&context, "host-");
        if publications.len() == 1 {
            break publications.into_iter().next().unwrap();
        }
        assert!(
            failure.0.try_wait().unwrap().is_none(),
            "failure fixture exited before publishing its restored grant:\n{}",
            bounded_output(&failure_output)
        );
        assert!(
            Instant::now() < publication_deadline,
            "remembered grant did not produce a live registration; {}; fixture output:\n{}",
            context_diagnostics(&context),
            bounded_output(&failure_output)
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let registration: Registration =
        serde_json::from_slice(&std::fs::read(&publication).unwrap()).unwrap();
    let incarnation = registration.host_incarnation.as_str();
    assert_eq!(
        publication.file_name().unwrap().to_string_lossy(),
        format!("host-{incarnation}.json")
    );
    assert_eq!(
        registration.root,
        root.join("project").canonicalize().unwrap(),
        "remembered grant restarted for a different workspace"
    );
    let pinned = PinnedProcess::open_peer(registration.pid).unwrap();
    assert_eq!(
        pinned.identity(),
        ProcessIdentity {
            pid: registration.pid,
            creation_time: registration.creation_time,
        }
    );
    assert!(pinned.is_alive().unwrap());
    assert!(
        publication.is_file(),
        "live context registration was not published at {}",
        publication.display()
    );
    std::fs::write(barrier.with_extension("release"), b"release").unwrap();
    let status = await_child(&mut failure, &failure_output, Duration::from_secs(30));
    assert!(status.success(), "{}", bounded_output(&failure_output));
    assert!(
        bounded_output(&failure_output)
            .contains("injected post-service frontend failure after service startup"),
        "failure fixture did not observe the injected frontend error:\n{}",
        bounded_output(&failure_output)
    );
    assert!(
        !pinned.is_alive().unwrap(),
        "failed editor process is still live"
    );
    assert!(
        !publication.exists(),
        "exact context publication survived joined frontend failure: {}",
        publication.display()
    );
}

#[cfg(debug_assertions)]
#[test]
#[ignore = "seeds a remembered context grant for the post-service failure fixture"]
fn public_windows_context_grant_seed_fixture() {
    let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
    assert_isolated_environment(&root);
    let context = root.join("context");
    let config_root = root.join("config");
    let (project, file, config) = prepare_editor_fixture(&root);
    let binary = Path::new(env!("CARGO_BIN_EXE_runyte"));
    let mut editor = Console::spawn(binary, &project, &config, &file);
    editor.until("a\u{00e7}\u{754c}\u{1f642}z");
    editor.until("NOR");
    editor.send(":context-access agent\r");
    grant_remembered_edit(&mut editor);
    wait_inventory(binary, &context, &config_root, 1);
    editor.send(":quit\r");
    editor.exit();
    drop(editor);
    wait_inventory(binary, &context, &config_root, 0);
}

#[cfg(debug_assertions)]
#[test]
#[ignore = "launches the real editor with an injected post-service frontend failure"]
fn public_windows_context_post_service_failure_fixture() {
    let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
    assert_isolated_environment(&root);
    let (project, file, config) = prepare_editor_fixture(&root);
    let binary = Path::new(env!("CARGO_BIN_EXE_runyte"));
    let mut editor = Console::spawn(binary, &project, &config, &file);
    assert_ne!(
        editor.exit_code(),
        Some(0),
        "post-service failure unexpectedly exited successfully"
    );
    let expected = "injected post-service frontend failure after service startup";
    assert!(
        String::from_utf8_lossy(&editor.output).contains(expected),
        "{}",
        String::from_utf8_lossy(&editor.output)
    );
    println!("{expected}");
}
