// SPDX-License-Identifier: MPL-2.0
use super::*;

#[test]
fn close_policy_prompt_markers_and_input_are_conservative() {
    let mut terminal = tests::session(80, 24);
    assert!(terminal.close_needs_confirmation());
    for bytes in [
        b"\x1b]133;B\x07".as_slice(),
        b"\x1b]133;C\x07",
        b"\x1b]133;D\x07",
        b"\x1b]133;A\x07",
        b"\x1b]133;unknown\x07",
    ] {
        terminal.feed(bytes);
        assert_eq!(terminal.emulator.prompt_ready, bytes == b"\x1b]133;B\x07");
        // A synthetic session cannot establish the OS no-job proof.
        assert!(terminal.close_needs_confirmation());
    }
    terminal.feed(b"\x1b]133;B\x07");
    assert!(terminal.emulator.prompt_ready);
    terminal.send_text("x");
    assert!(!terminal.emulator.prompt_ready);
    terminal.feed(b"\x1b]133;B\x07");
    assert!(terminal.close_needs_confirmation());
    terminal.feed(b"\x1b]133;B\x07\x1bc");
    assert!(!terminal.emulator.prompt_ready);
    terminal.exit = Some(Some(0));
    assert!(!terminal.close_needs_confirmation());
}

#[cfg(target_os = "linux")]
#[test]
fn close_policy_real_shell_idle_and_background_job() {
    use std::{
        fs,
        time::{Duration, Instant},
    };
    for background in [false, true] {
        let root = crate::test_support::TestRuntimeRoot::new("close-policy").unwrap();
        let program = root.join("shell");
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
            &program,
        )
        .unwrap();
        let behavior = if background {
            "sleep 60 &\nprintf '\\033]133;B\\007'\nread answer\n"
        } else {
            "printf '\\033]133;B\\007'\nread answer\n"
        };
        fs::write(root.join("shell.behavior"), behavior).unwrap();
        let mut arguments = vec!["-i".to_owned(), "PATH=/usr/bin:/bin".to_owned()];
        arguments.extend(
            crate::test_support::user_environment(&root)
                .map(|(key, value)| format!("{key}={}", value.display())),
        );
        arguments.push(program.to_string_lossy().into_owned());
        let mut terminals = TerminalSessions::new();
        let mut events = terminals.take_events().unwrap();
        let id = terminals
            .open(
                TerminalRequest {
                    program: "/usr/bin/env".into(),
                    arguments,
                    directory: root.to_path_buf(),
                    label: "fixture".into(),
                },
                80,
                24,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !terminals.get(id).unwrap().emulator.prompt_ready {
            drain(&mut events, |output| terminals.apply(output));
            assert!(Instant::now() < deadline, "shell did not report prompt");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            terminals.get(id).unwrap().close_needs_confirmation(),
            background
        );
        // No output is needed to turn an idle prompt into an uncertain state.
        terminals.get_mut(id).unwrap().send_text("pending");
        assert!(terminals.get(id).unwrap().close_needs_confirmation());
        // Even a delayed marker cannot rearm automatic closing after input.
        terminals.get_mut(id).unwrap().feed(b"\x1b]133;B\x07");
        assert!(terminals.get(id).unwrap().close_needs_confirmation());
        terminals.close(id);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn close_policy_foreground_job_overrides_stale_prompt_report() {
    use std::time::{Duration, Instant};
    let root = crate::test_support::TestRuntimeRoot::new("close-foreground").unwrap();
    let mut arguments = vec!["-i".to_owned(), "PATH=/usr/bin:/bin".to_owned()];
    arguments.extend(
        crate::test_support::user_environment(&root)
            .map(|(key, value)| format!("{key}={}", value.display())),
    );
    arguments.extend(
        [
            "/bin/bash",
            "--noprofile",
            "--norc",
            "-i",
            "-c",
            r"printf '\033]133;B\007'; sleep 60; :",
        ]
        .map(str::to_owned),
    );
    let mut terminals = TerminalSessions::new();
    let mut events = terminals.take_events().unwrap();
    let id = terminals
        .open(
            TerminalRequest {
                program: "/usr/bin/env".into(),
                arguments,
                directory: root.to_path_buf(),
                label: "foreground fixture".into(),
            },
            80,
            24,
        )
        .unwrap();
    let pid = terminals.get(id).unwrap().process_id().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        drain(&mut events, |output| terminals.apply(output));
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let foreground = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(5)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        if foreground > 0
            && foreground != pid as i32
            && terminals.get(id).unwrap().emulator.prompt_ready
        {
            break;
        }
        assert!(Instant::now() < deadline, "job never became foreground");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(terminals.get(id).unwrap().close_needs_confirmation());
    terminals.close(id);
}
