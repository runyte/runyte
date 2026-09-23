// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    terminal::pty::{Pty, PtyEvent},
    test_support::TestRuntimeRoot,
};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const FIXTURE: &str = "native_standalone_wait_fixture";
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

struct FixtureChild(Child);
impl Drop for FixtureChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
fn windows_wait_owns_a_standalone_editor_until_explicit_quit() {
    let root = TestRuntimeRoot::new("windows-wait").unwrap();
    let output = std::fs::File::create(root.join("output.log")).unwrap();
    let mut fixture = FixtureChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .env("RUNYTE_WAIT_ROOT", root.path())
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(status) = fixture.0.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                std::fs::read_to_string(root.join("output.log")).unwrap()
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native wait fixture timed out: {}",
            std::fs::read_to_string(root.join("output.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Console {
    child: Pty,
    events: mpsc::Receiver<PtyEvent>,
    screen: runyte::terminal::emulator::Emulator,
    transcript: Vec<u8>,
}

impl Console {
    fn spawn(root: &Path, config: &Path, targets: &[&str]) -> Self {
        let (sender, events) = mpsc::channel();
        let mut args = vec![
            "--wait".to_owned(),
            "--config".to_owned(),
            config.to_str().unwrap().to_owned(),
            "--project-root".to_owned(),
            root.to_str().unwrap().to_owned(),
            "--".to_owned(),
        ];
        args.extend(targets.iter().map(|target| (*target).to_owned()));
        let child = Pty::spawn(
            Path::new(env!("CARGO_BIN_EXE_runyte")).as_os_str(),
            &args,
            root,
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
            screen: runyte::terminal::emulator::Emulator::new(120, 30),
            transcript: Vec::new(),
        }
    }

    fn output(&mut self, bytes: &[u8]) {
        assert!(self.transcript.len() + bytes.len() <= OUTPUT_LIMIT);
        self.transcript.extend_from_slice(bytes);
        self.screen.feed(bytes);
    }

    fn until(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let screen = (0..self.screen.rows())
                .filter_map(|row| self.screen.grid().line(row))
                .map(|line| {
                    line.iter()
                        .filter(|cell| cell.width != 0)
                        .map(|cell| cell.text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            if screen.contains(marker) {
                assert!(self.child.finished().is_none());
                return;
            }
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => self.output(&bytes),
                event => panic!(
                    "waiting for {marker:?}, got {event:?}: {}",
                    String::from_utf8_lossy(&self.transcript)
                ),
            }
        }
    }

    fn send(&self, text: &str) {
        assert!(self.child.write(text.as_bytes().to_vec()));
    }

    fn exit(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => self.output(&bytes),
                Ok(PtyEvent::Exited(code)) => return code,
                error => panic!("{error:?}: {}", String::from_utf8_lossy(&self.transcript)),
            }
        }
    }
}

#[test]
#[ignore = "compiled fixture relaunched with isolated configuration and cache"]
fn native_standalone_wait_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_WAIT_ROOT").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    assert_eq!(
        std::env::var_os("XDG_CACHE_HOME").unwrap(),
        root.join("cache")
    );
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
    );
    let project = root.join("project");
    let config = root.join("config/config.yaml");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    // An explicit --wait must override the bare-launch persistent preference.
    std::fs::write(
        &config,
        "lsp:\n  enable: false\nworkspace:\n  mode: persistent\n",
    )
    .unwrap();
    let first = "-first [note] café.txt";
    let second = "second note.txt";
    std::fs::write(project.join(first), "WAIT_FIRST_MARKER\n").unwrap();
    std::fs::write(project.join(second), "WAIT_SECOND_MARKER\n").unwrap();

    let mut editor = Console::spawn(&project, &config, &[first, second]);
    editor.until("WAIT_FIRST_MARKER");
    editor.until("NOR");
    editor.send("i");
    editor.until("INS");
    editor.send("\x1b[200~saved \x1b[201~");
    editor.until("saved WAIT_FIRST_MARKER");
    editor.send("\x1b");
    editor.until("NOR");
    editor.send(":quit\r");
    editor.until("unsaved changes");
    assert_eq!(
        std::fs::read_to_string(project.join(first)).unwrap(),
        "WAIT_FIRST_MARKER\n"
    );
    // Closing one requested buffer does not finish a standalone --wait.
    editor.send(":wbc\r");
    editor.until("WAIT_SECOND_MARKER");
    assert_eq!(
        std::fs::read_to_string(project.join(first)).unwrap(),
        "saved WAIT_FIRST_MARKER\n"
    );
    editor.send(":quit\r");
    assert_eq!(editor.exit(), Some(0));

    let mut editor = Console::spawn(&project, &config, &[second]);
    editor.until("WAIT_SECOND_MARKER");
    editor.send("i");
    editor.until("INS");
    editor.send("\x1b[200~discard \x1b[201~");
    editor.until("discard WAIT_SECOND_MARKER");
    editor.send("\x1b");
    editor.until("NOR");
    editor.send(":quit!\r");
    assert_eq!(editor.exit(), Some(0));
    assert_eq!(
        std::fs::read_to_string(project.join(second)).unwrap(),
        "WAIT_SECOND_MARKER\n"
    );

    let mut editor = Console::spawn(&project, &config, &[second]);
    editor.until("WAIT_SECOND_MARKER");
    editor.child.terminate();
    assert_ne!(editor.exit(), Some(0));

    // Invalid launch requests fail before entering an editor, also without a
    // terminal. Each process gets explicit fixture-owned environment roots.
    for args in [
        vec!["--wait"],
        vec!["--wait", "+2", second],
        vec!["--wait", "--standalone", second],
        vec!["--wait", "--config", "missing.yaml", second],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
            .args(args)
            .current_dir(&project)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
    assert!(
        !project
            .join(".runyte/host/endpoint.json")
            .try_exists()
            .unwrap(),
        "standalone --wait must not launch a persistent host"
    );
}
