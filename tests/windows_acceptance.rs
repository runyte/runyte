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
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
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
    fn spawn(program: &Path, args: &[String], root: &Path, columns: u16) -> Self {
        let (sender, events) = mpsc::channel();
        let child = Pty::spawn(program.as_os_str(), args, root, columns, 30, move |event| {
            let _ = sender.send(event);
        })
        .unwrap();
        Self {
            child,
            events,
            output: Vec::new(),
            transcript: Vec::new(),
            screen: runyte::terminal::emulator::Emulator::new(usize::from(columns), 30),
        }
    }
    fn until(&mut self, needle: &str) {
        self.until_with_row_separator(needle, "\n");
    }
    fn until_prompt(&mut self, needle: &str) {
        // Setup prompts precede the editor's framed screen and may wrap in
        // the middle of their marker when the temporary path is long.
        self.until_with_row_separator(needle, "");
    }
    fn until_with_row_separator(&mut self, needle: &str, separator: &str) {
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
            .join(separator)
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
    assert_eq!(
        std::env::var_os("XDG_CACHE_HOME").unwrap(),
        root.join("cache")
    );
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
    );
    let project = root.join("project");
    // Force the setup confirmation marker to span two rows, as happened with
    // the longer temp path on CI. Adjust geometry rather than lengthening cwd.
    let prompt_prefix = format!(
        "Save Runyte project data in {}? ",
        project.join(".runyte").display()
    );
    let columns =
        u16::try_from(unicode_width::UnicodeWidthStr::width(prompt_prefix.as_str()) + 2).unwrap();
    let config = root.join("config/config.yaml");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "lsp:\n  enable: false\n").unwrap();
    let file = project.join("unicode note.txt");
    std::fs::write(&file, "original\r\n").unwrap();
    let mut editor = Console::spawn(
        Path::new(env!("CARGO_BIN_EXE_runyte")),
        &[
            "--standalone".into(),
            "--config".into(),
            config.display().to_string(),
            file.display().to_string(),
        ],
        &project,
        columns,
    );
    editor.until_prompt("Project directory [");
    editor.send("\r");
    editor.until_prompt("[y/N]:");
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
    editor.until("wrote");
    // Inspect the completed save, after the editor acknowledges it. Paste
    // preserves LF bytes alongside the existing CRLF without normalization.
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "café 😀\nsecond-lineoriginal\r\n"
    );
    editor.send("%:pipe [Console]::Write([Console]::In.ReadToEnd().ToUpperInvariant())\r");
    editor.until("CAFÉ");
    editor.until("SECOND-LINEORIGINAL");
    // The whole filter is one undo transaction; its unsaved result must not
    // change the previously saved bytes on disk.
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "café 😀\nsecond-lineoriginal\r\n"
    );
    editor.send("u");
    editor.until("café");
    editor.send(":quit\r");
    editor.exit();
    assert!(
        !String::from_utf8_lossy(&editor.transcript).contains("Private diagnostic log storage")
    );
}
