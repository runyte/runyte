// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    terminal::pty::{Pty, PtyEvent},
    test_support::TestRuntimeRoot,
};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const FIXTURE: &str = "native_editor_console_fixture";

#[test]
fn real_editor_paste_and_save() {
    let root = TestRuntimeRoot::new("windows-acceptance").unwrap();
    let output = std::fs::File::create(root.join("output.log")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("RUNYTE_ACCEPTANCE_ROOT", root.path())
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!(
                "native acceptance timed out: {}",
                std::fs::read_to_string(root.join("output.log")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(root.join("output.log")).unwrap()
    );
}

struct Console {
    child: Pty,
    events: mpsc::Receiver<PtyEvent>,
    output: Vec<u8>,
    transcript: Vec<u8>,
    screen: runyte::terminal::emulator::Emulator,
}
impl Console {
    fn spawn(program: &Path, args: &[String], root: &Path) -> Self {
        let (sender, events) = mpsc::channel();
        let child = Pty::spawn(program.as_os_str(), args, root, 120, 30, move |event| {
            let _ = sender.send(event);
        })
        .unwrap();
        Self {
            child,
            events,
            output: Vec::new(),
            transcript: Vec::new(),
            screen: runyte::terminal::emulator::Emulator::new(120, 30),
        }
    }
    fn until(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !(0..self.screen.rows())
            .filter_map(|row| self.screen.grid().line(row))
            .map(|line| {
                line.iter()
                    .filter(|cell| cell.width != 0)
                    .map(|cell| cell.text())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .contains(needle)
        {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => {
                    self.screen.feed(&bytes);
                    self.transcript.extend_from_slice(&bytes);
                    self.output.extend(bytes);
                }
                event => panic!(
                    "waiting for {needle:?}, got {event:?}: {}",
                    String::from_utf8_lossy(&self.output)
                ),
            }
        }
        self.output.clear();
    }
    fn send(&self, text: &str) {
        assert!(self.child.write(text.as_bytes().to_vec()));
    }
    fn exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => {
                    self.transcript.extend_from_slice(&bytes);
                    self.output.extend(bytes);
                }
                Ok(PtyEvent::Exited(code)) => {
                    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&self.output));
                    break;
                }
                error => panic!("{error:?}: {}", String::from_utf8_lossy(&self.output)),
            }
        }
    }
}

#[test]
#[ignore = "reexecuted with fixture-owned configuration by the acceptance test"]
fn native_editor_console_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("RUNYTE_ACCEPTANCE_ROOT").unwrap());
    // Every descendant inherits this fixture-owned configuration from the
    // explicit Command builder above, without global environment mutation.
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    let file = root.join("unicode note.txt");
    std::fs::write(&file, "original\r\n").unwrap();
    let mut editor = Console::spawn(
        Path::new(env!("CARGO_BIN_EXE_runyte")),
        &["--standalone".into(), file.display().to_string()],
        &root,
    );
    editor.until("Project directory [");
    editor.send("\r");
    editor.until("[y/N]:");
    editor.send("y\r");
    editor.until("original");
    editor.until("NOR");
    // In Normal mode this must never execute :quit! or its following line.
    // Normal-mode text events are ignored by the shared editor input contract.
    editor.send("\x1b[200~:quit!\nnormal-paste-marker\x1b[201~");
    editor.send("i");
    editor.until("INS");
    editor.send("\x1b[200~café 😀\nsecond-line\x1b[201~");
    editor.until("second-line");
    editor.send("\x1b");
    editor.until("NOR");
    editor.send(":write\r");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let saved = std::fs::read_to_string(&file).unwrap();
        if saved.contains("café 😀") {
            // Paste preserves its LF bytes; the existing file's CRLF remains
            // intact instead of normalizing the entire buffer during save.
            assert_eq!(saved, "café 😀\nsecond-lineoriginal\r\n");
            break;
        }
        assert!(Instant::now() < deadline, "file was not saved: {saved:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    editor.send(":quit\r");
    editor.exit();
    assert!(
        !String::from_utf8_lossy(&editor.transcript).contains("Private diagnostic log storage")
    );
}
