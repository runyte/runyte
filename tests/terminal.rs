// SPDX-License-Identifier: MPL-2.0

//! End-to-end behaviour of a terminal pane: a real child process on a real
//! pseudoterminal, driven through the ordinary editor input boundary and
//! rendered through the ordinary frame path.
//!
//! Every child here is a fixed program with fixed arguments — `cat`, `echo`,
//! `printf` — rather than `$SHELL`, so nothing reads the person's rc files and
//! nothing depends on which shell they use.

#![cfg(unix)]

use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use ratatui::{Terminal, backend::TestBackend};
use runyte::{
    app::App,
    clipboard::SystemClipboard,
    command::Mode,
    config::Config,
    input::{
        InputEvent, KeyCode, KeyStroke, Modifiers, PointerButton, PointerEvent, PointerEventKind,
    },
    key_hints::KeyHintState,
    snapshot::OverlayKind,
    terminal::{self, OUTPUT_QUEUE, TerminalEvents, TerminalOutput},
    test_support::TestRuntimeRoot,
    ui,
};

struct MemoryClipboard(Arc<Mutex<String>>);

impl SystemClipboard for MemoryClipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn write(&mut self, text: &str) -> anyhow::Result<()> {
        *self.0.lock().unwrap() = text.to_owned();
        Ok(())
    }
}

/// A terminal pane wired to the output stream the editor would normally have
/// handed to an event loop.
struct Session {
    app: App,
    output: TerminalEvents,
}

impl Session {
    fn start(command: &str) -> Self {
        Self::start_with(Config::default(), command)
    }

    fn start_with(config: Config, command: &str) -> Self {
        Self::start_with_file(config, command, None)
    }

    fn start_with_file(config: Config, command: &str, file: Option<PathBuf>) -> Self {
        Self::start_from_app(App::new(config, file).unwrap(), command)
    }

    fn start_from_app(mut app: App, command: &str) -> Self {
        let output = app
            .terminals
            .take_events()
            .expect("the output stream is available before any loop claims it");
        // Give the pane a shape before the child starts, the way a drawn frame
        // would, so the size the child sees is the one under test.
        render(&mut app, 60, 12);
        type_colon(&mut app, &format!("terminal {command}"));
        Self { app, output }
    }

    /// Drains child output until `ready` holds, or gives up.
    ///
    /// A child writes when it is scheduled, not when a test asks, so every
    /// assertion about what is on screen has to wait for it.
    fn settle(&mut self, ready: impl Fn(&App) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if ready(&self.app) {
                return true;
            }
            match self.output.try_recv() {
                Ok(output) => self.app.apply_terminal_output(output),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        ready(&self.app)
    }

    fn screen(&mut self, width: u16, height: u16) -> String {
        render(&mut self.app, width, height)
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.app.handle_key(KeyStroke::char(character)).unwrap();
        }
    }

    fn press(&mut self, code: KeyCode) {
        self.app.handle_key(KeyStroke::plain(code)).unwrap();
    }

    /// Types a colon command, leaving the terminal's input first.
    ///
    /// A terminal in INSERT mode owns `:` like every other key, so a test that
    /// typed one there would be sending it to the child.
    fn colon(&mut self, command: &str) {
        self.leave_input();
        type_colon(&mut self.app, command);
    }

    fn leave_input(&mut self) {
        if self.app.mode == Mode::Insert {
            self.app
                .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
                .unwrap();
        }
        if self
            .app
            .active_terminal()
            .and_then(|id| self.app.terminals.get(id))
            .is_some_and(|terminal| !terminal.reviewing())
        {
            self.app
                .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
                .unwrap();
        }
    }
}

/// Runs a colon command the way a person does, through the palette.
fn type_colon(app: &mut App, command: &str) {
    app.handle_key(KeyStroke::char(':')).unwrap();
    for character in command.chars() {
        app.handle_key(KeyStroke::char(character)).unwrap();
    }
    app.handle_key(KeyStroke::plain(KeyCode::Enter)).unwrap();
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let hints = KeyHintState::default();
    terminal
        .draw(|frame| {
            let prepared = app.prepare_view(ui::frame_geometry(frame.area()));
            let snapshot = app.snapshot(&prepared);
            ui::render_exact_colors_for_test(frame, app, &snapshot, &hints);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|row| {
            (0..width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The panes of a frame the size `render` draws, without drawing it.
fn panes(app: &mut App, width: u16, height: u16) -> Vec<runyte::snapshot::PaneSnapshot> {
    let prepared = app.prepare_view(ui::frame_geometry(ratatui::layout::Rect::new(
        0, 0, width, height,
    )));
    app.snapshot(&prepared).panes
}

fn terminal_text(app: &App) -> String {
    let id = app.active_terminal().expect("the pane shows a terminal");
    app.terminals
        .get(id)
        .expect("the session exists")
        .plain_text()
}

fn review_selection(app: &mut App, id: runyte::terminal::TerminalId) -> String {
    app.terminals
        .get_mut(id)
        .expect("the session exists")
        .review_selection_text()
}

fn terminal_text_by_id(app: &App, id: runyte::terminal::TerminalId) -> String {
    app.terminals
        .get(id)
        .expect("the session exists")
        .plain_text()
}

#[test]
fn a_child_runs_in_the_pane_and_its_output_is_drawn() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "terminal pane works\r\n"; cat'"#);
    assert!(session.settle(|app| terminal_text(app).contains("terminal pane works")));
    assert!(session.screen(60, 12).contains("terminal pane works"));
}

#[test]
fn a_git_merge_wait_editor_exiting_inside_a_terminal_returns_to_its_shell() {
    for close in ["q", "c"] {
        let sandbox = TestRuntimeRoot::new("nested-editor").unwrap();
        let project = sandbox.create_private_dir("project").unwrap();
        let cache = sandbox.create_private_dir("cache").unwrap();
        let git = |arguments: &[&str]| {
            assert!(
                Command::new("git")
                    .args(arguments)
                    .current_dir(&project)
                    .status()
                    .unwrap()
                    .success(),
                "git {arguments:?} failed"
            );
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Runyte Test"]);
        git(&["config", "user.email", "runyte@example.invalid"]);
        std::fs::write(project.join("base"), "base\n").unwrap();
        git(&["add", "base"]);
        git(&["commit", "-q", "-m", "base"]);
        git(&["checkout", "-q", "-b", "side"]);
        std::fs::write(project.join("side"), "side\n").unwrap();
        git(&["add", "side"]);
        git(&["commit", "-q", "-m", "side"]);
        git(&["checkout", "-q", "main"]);
        let runyte = env!("CARGO_BIN_EXE_runyte");
        let command = format!(
            "/bin/sh -c 'cd {project}; XDG_RUNTIME_DIR={runtime} XDG_CACHE_HOME={cache} GIT_EDITOR=\"{runyte} --wait\" git merge --no-ff side; status=$?; XDG_RUNTIME_DIR={runtime} XDG_CACHE_HOME={cache} {runyte} --session-stop --force >/dev/null 2>&1; printf \"git-merge-finished:%s\\r\\n\" \"$status\"; cat'",
            runtime = sandbox.display(),
            cache = cache.display(),
            project = project.display(),
        );
        let mut session = Session::start(&command);

        let drawn = session.settle(|app| {
            app.active_terminal()
                .and_then(|id| app.terminals.get(id))
                .is_some_and(|terminal| terminal.plain_text().contains("Merge branch 'side'"))
        });
        assert!(
            drawn,
            "nested Runyte did not draw for :{close}; status: {:?}; terminal: {:?}",
            session.app.status,
            session
                .app
                .active_terminal()
                .and_then(|id| session.app.terminals.get(id))
                .map(|terminal| terminal.plain_text())
        );
        session.type_text(":");
        session.type_text(close);
        session.press(KeyCode::Enter);

        assert!(
            session.settle(|app| {
                app.active_terminal()
                    .and_then(|id| app.terminals.get(id))
                    .is_some_and(|terminal| terminal.plain_text().contains("git-merge-finished:0"))
            }),
            "integrated terminal did not return to its shell after Git's editor closed with :{close}"
        );
        let id = session
            .app
            .active_terminal()
            .expect("the integrated terminal remains in its pane");
        assert!(session.app.terminals.get(id).unwrap().live());
    }
}

#[test]
fn a_child_can_discover_the_effective_default_background() {
    let command = r#"/bin/sh -c 'stty raw -echo; printf "\033]11;?\033\\"; answer=$(dd bs=1 count=25 2>/dev/null); expected=$(printf "\033]11;rgb:2828/2a2a/2f2f\033\\"); if [ "$answer" = "$expected" ]; then printf default-background-ok; else printf default-background-wrong; fi; cat'"#;
    let mut session = Session::start(command);

    assert!(
        session.settle(|app| terminal_text(app).contains("default-background-ok")),
        "terminal output: {:?}",
        terminal_text(&session.app)
    );
    assert!(!terminal_text(&session.app).contains("default-background-wrong"));
}

#[test]
fn control_backslash_steps_from_terminal_input_through_normal_to_review() {
    for exit in [KeyStroke::ctrl('\\'), KeyStroke::ctrl('4')] {
        let mut session = Session::start("/bin/cat");
        let terminal = session.app.active_terminal().unwrap();
        assert_eq!(session.app.mode, Mode::Insert);
        assert!(!session.app.terminals.get(terminal).unwrap().reviewing());

        session.app.handle_key(exit).unwrap();
        assert_eq!(session.app.mode, Mode::Normal);
        assert!(!session.app.terminals.get(terminal).unwrap().reviewing());

        // The second press explicitly freezes the live output for review.
        session.app.handle_key(exit).unwrap();
        assert_eq!(session.app.mode, Mode::Normal);
        assert!(session.app.terminals.get(terminal).unwrap().reviewing());

        // `i` goes back in rather than inserting into the buffer behind the pane.
        session.type_text("i");
        assert_eq!(session.app.mode, Mode::Insert);
        assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    }
}

#[test]
fn typing_in_insert_mode_reaches_the_child() {
    let mut session = Session::start("/bin/cat");
    session.type_text("hello");
    session.press(KeyCode::Enter);
    assert!(session.settle(|app| terminal_text(app).contains("hello")));
}

#[test]
fn control_w_starts_pane_navigation_without_leaving_terminal_input() {
    let mut session = Session::start("/bin/sh -c 'stty raw -echo; printf ready; cat -v'");
    assert!(session.settle(|app| terminal_text(app).contains("ready")));
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    assert_eq!(session.app.mode, Mode::Insert);
    assert_eq!(session.app.pending_sequence().to_string(), "Ctrl-w");
    session.press(KeyCode::Escape);
    assert_eq!(session.app.mode, Mode::Insert);
    assert!(session.app.pending_sequence().is_empty());

    session.type_text("x");
    session.press(KeyCode::Enter);
    assert!(
        session.settle(|app| terminal_text(app).contains('x')),
        "mode {:?}, status {:?}, text {:?}",
        session.app.mode,
        session.app.status,
        terminal_text(&session.app)
    );
}

#[test]
fn control_w_fullscreen_and_zen_preserve_every_terminal_mode() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();

    // Insert uses the terminal-scoped window bindings without first entering
    // Normal or sending either key to the child.
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    assert_eq!(session.app.mode, Mode::Insert);
    session.type_text("f");
    assert_eq!(session.app.mode, Mode::Insert);
    assert_eq!(session.app.status, "full-screen view enabled");
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("f");
    assert_eq!(session.app.status, "full-screen view disabled");

    // Live Normal keeps output live, and opening the prefix itself does not
    // capture a review snapshot.
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    assert_eq!(session.app.mode, Mode::Normal);
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    assert_eq!(session.app.mode, Mode::Normal);
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    session.type_text("z");
    assert_eq!(session.app.mode, Mode::Normal);
    assert_eq!(session.app.status, "zen mode enabled at 100 columns");
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());

    // Captured Normal/review and Select retain both the snapshot and mode.
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("f");
    assert_eq!(session.app.mode, Mode::Normal);
    assert_eq!(session.app.status, "full-screen view enabled");
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());

    session.type_text("v");
    assert_eq!(session.app.mode, Mode::Select);
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    assert_eq!(session.app.mode, Mode::Select);
    session.type_text("z");
    assert_eq!(session.app.mode, Mode::Select);
    assert_eq!(session.app.status, "zen mode enabled at 100 columns");
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());
}

#[test]
fn control_w_focus_moves_directly_from_terminal_insert_without_sending_input() {
    let mut session = Session::start("/bin/sh -c 'stty raw -echo; printf ready; cat -v'");
    assert!(session.settle(|app| terminal_text(app).contains("ready")));
    let terminal = session.app.active_terminal().unwrap();
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("v");
    assert!(session.app.active_terminal().is_none());
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    render(&mut session.app, 60, 12);

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("h");
    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.mode, Mode::Insert);

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("l");
    assert!(session.app.active_terminal().is_none());
    assert_eq!(session.app.mode, Mode::Normal);
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    assert!(!terminal_text_by_id(&session.app, terminal).contains("^W"));
}

/// With `editor.fast_pane_keys` on, the four keys leave a live terminal the
/// way `Ctrl-w h` already does — the child never sees them.
#[test]
fn fast_pane_keys_move_out_of_terminal_input_without_reaching_the_child() {
    let mut config = Config::default();
    config.editor.fast_pane_keys = true;
    let mut session =
        Session::start_with(config, "/bin/sh -c 'stty raw -echo; printf ready; cat -v'");
    assert!(session.settle(|app| terminal_text(app).contains("ready")));
    let terminal = session.app.active_terminal().unwrap();
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("v");
    assert!(session.app.active_terminal().is_none());
    render(&mut session.app, 60, 12);

    // Into the terminal and back out again, one keystroke each way. A terminal
    // destination starts input immediately, and the same fast key can leave
    // it without an intermediate mode change.
    session.app.handle_key(KeyStroke::ctrl('h')).unwrap();
    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.mode, Mode::Insert);

    session.app.handle_key(KeyStroke::ctrl('l')).unwrap();
    assert!(session.app.active_terminal().is_none());
    assert_eq!(session.app.mode, Mode::Normal);

    let echoed = terminal_text_by_id(&session.app, terminal);
    assert!(!echoed.contains("^H"), "{echoed}");
    assert!(!echoed.contains("^L"), "{echoed}");
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
}

#[test]
fn pane_keys_dispatch_over_a_read_only_buffer_hidden_by_the_terminal() {
    for fast in [false, true] {
        let mut config = Config::default();
        config.editor.fast_pane_keys = fast;
        let mut app = App::new(config, None).unwrap();
        type_colon(&mut app, "about");
        assert!(app.active_buffer().is_read_only());

        let mut session = Session::start_from_app(app, "/bin/cat");
        let terminal = session.app.active_terminal().unwrap();
        session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
        session.type_text("v");
        assert!(session.app.active_terminal().is_none());
        render(&mut session.app, 60, 12);

        if fast {
            session.app.handle_key(KeyStroke::ctrl('h')).unwrap();
        } else {
            session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
            assert_eq!(session.app.mode, Mode::Normal);
            session.type_text("h");
        }
        assert_eq!(session.app.active_terminal(), Some(terminal));
        assert_eq!(session.app.mode, Mode::Insert);

        if fast {
            session.app.handle_key(KeyStroke::ctrl('l')).unwrap();
        } else {
            session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
            assert_eq!(session.app.mode, Mode::Insert);
            session.type_text("l");
        }
        assert!(session.app.active_terminal().is_none());
        assert_eq!(session.app.mode, Mode::Normal);
        assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    }
}

/// Both directional spellings activate live input on a terminal destination,
/// and can move away again without an intermediate mode change.
#[test]
fn directional_pane_keys_focus_another_terminal_in_insert() {
    for fast in [false, true] {
        for (split, backward, forward) in [('v', 'h', 'l'), ('s', 'k', 'j')] {
            let mut config = Config::default();
            config.editor.fast_pane_keys = fast;
            let mut session = Session::start_with(config, "/bin/cat");
            let first = session.app.active_terminal().unwrap();

            session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
            session.type_text(&split.to_string());
            assert!(session.app.active_terminal().is_none());
            type_colon(&mut session.app, "terminal /bin/cat");
            let second = session.app.active_terminal().unwrap();
            assert_ne!(first, second);
            render(&mut session.app, 60, 12);

            if fast {
                session.app.handle_key(KeyStroke::ctrl(backward)).unwrap();
            } else {
                session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
                session.type_text(&backward.to_string());
            }

            assert_eq!(session.app.active_terminal(), Some(first));
            assert_eq!(session.app.mode, Mode::Insert);
            assert!(!session.app.terminals.get(first).unwrap().reviewing());
            assert!(!session.app.terminals.get(second).unwrap().reviewing());

            if fast {
                session.app.handle_key(KeyStroke::ctrl(forward)).unwrap();
            } else {
                session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
                session.type_text(&forward.to_string());
            }

            assert_eq!(session.app.active_terminal(), Some(second));
            assert_eq!(session.app.mode, Mode::Insert);
            assert!(!session.app.terminals.get(first).unwrap().reviewing());
            assert!(!session.app.terminals.get(second).unwrap().reviewing());
        }
    }
}

/// Off, which is the default, the same keys are the child's own. `Ctrl-l`
/// clearing the screen is the one people notice, so it is the one asserted.
#[test]
fn without_fast_pane_keys_the_child_still_receives_control_h_and_l() {
    let mut session = Session::start("/bin/sh -c 'stty raw -echo; printf ready; cat -v'");
    assert!(session.settle(|app| terminal_text(app).contains("ready")));
    let terminal = session.app.active_terminal().unwrap();

    session.app.handle_key(KeyStroke::ctrl('l')).unwrap();
    assert!(session.settle(|app| terminal_text_by_id(app, terminal).contains("^L")));
    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.mode, Mode::Insert);
}

/// Review is the frozen half of a terminal pane: the child has stopped
/// painting and the keys move a caret over a still image. Graying the text is
/// what tells the two halves apart on screen.
#[test]
fn a_terminal_is_dimmed_while_it_is_under_review_and_colourful_while_it_is_live() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();

    assert_eq!(session.app.mode, Mode::Insert);
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    assert!(
        panes(&mut session.app, 60, 12)
            .iter()
            .all(|pane| !pane.dimmed)
    );

    session.leave_input();
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());
    assert!(
        panes(&mut session.app, 60, 12)
            .iter()
            .all(|pane| pane.dimmed)
    );

    session.type_text("i");
    assert_eq!(session.app.mode, Mode::Insert);
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    assert!(
        panes(&mut session.app, 60, 12)
            .iter()
            .all(|pane| !pane.dimmed)
    );
}

/// Leaving a terminal for another pane keeps the child live, so the terminal
/// it left behind stays in its own colours.
#[test]
fn moving_away_from_a_live_terminal_does_not_dim_it() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("v");
    render(&mut session.app, 60, 12);
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("l");

    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());
    assert!(
        panes(&mut session.app, 60, 12)
            .iter()
            .all(|pane| !pane.dimmed)
    );
}

#[test]
fn control_w_focus_preserves_review_until_an_insert_key() {
    for (split, suffix) in [('v', 'h'), ('s', 'k'), ('v', 'w')] {
        let mut session = Session::start("/bin/cat");
        let first = session.app.active_terminal().unwrap();

        // This was the layout-building sequence that made the bug appear
        // positional: the first terminal retained review while a split gained
        // a second live terminal.
        session.leave_input();
        assert!(session.app.terminals.get(first).unwrap().reviewing());
        session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
        session.type_text(&split.to_string());
        type_colon(&mut session.app, "terminal /bin/cat");
        let second = session.app.active_terminal().unwrap();
        assert_ne!(first, second);
        render(&mut session.app, 60, 12);

        session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
        session.type_text(&suffix.to_string());
        assert_eq!(session.app.active_terminal(), Some(first));
        assert_eq!(session.app.mode, Mode::Normal);
        assert!(session.app.terminals.get(first).unwrap().reviewing());

        session.type_text("i");
        assert_eq!(session.app.mode, Mode::Insert);
        assert!(!session.app.terminals.get(first).unwrap().reviewing());
        session.type_text("x");
        assert!(session.settle(|app| terminal_text_by_id(app, first).contains('x')));
    }
}

#[test]
fn pane_swap_moves_a_terminal_session_and_preserves_its_review() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();
    let original_pane = session.app.active_pane;
    session.leave_input();
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("v");
    let document_pane = session.app.active_pane;
    assert_ne!(document_pane, original_pane);

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("x");
    assert_eq!(session.app.active_pane, original_pane);
    assert_eq!(session.app.active_terminal(), None);
    assert_eq!(session.app.terminal_of_pane(document_pane), Some(terminal));
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());

    render(&mut session.app, 60, 12);
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("l");
    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.mode, Mode::Normal);

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("x");
    assert_eq!(session.app.active_pane, original_pane);
    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.terminal_of_pane(document_pane), None);
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());
}

#[test]
fn terminal_insert_swap_keeps_the_live_child_and_resizes_at_its_new_geometry() {
    let mut session = Session::start("/bin/sh -c 'stty raw -echo; printf ready; cat -v'");
    assert!(session.settle(|app| terminal_text(app).contains("ready")));
    let terminal = session.app.active_terminal().unwrap();
    let terminal_pane = session.app.active_pane;
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("v");
    let document_pane = session.app.active_pane;
    render(&mut session.app, 80, 16);
    type_colon(&mut session.app, "resize-left + 10");
    render(&mut session.app, 80, 16);

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("h");
    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.mode, Mode::Insert);
    let narrow_columns = session.app.terminals.get(terminal).unwrap().columns();

    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("x");
    assert_eq!(session.app.active_pane, document_pane);
    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.terminal_of_pane(terminal_pane), None);
    assert_eq!(session.app.mode, Mode::Insert);
    assert!(!session.app.terminals.get(terminal).unwrap().reviewing());

    render(&mut session.app, 80, 16);
    let wide_columns = session.app.terminals.get(terminal).unwrap().columns();
    assert!(
        wide_columns > narrow_columns,
        "{narrow_columns} -> {wide_columns}"
    );
    session
        .app
        .handle_input(InputEvent::Text("still alive".to_owned()))
        .unwrap();
    assert!(session.settle(|app| terminal_text_by_id(app, terminal).contains("still alive")));
}

#[test]
fn control_w_from_document_insert_preserves_terminal_review() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();

    session.leave_input();
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("v");
    render(&mut session.app, 60, 12);

    session.type_text("i");
    assert_eq!(session.app.mode, Mode::Insert);
    assert!(session.app.active_terminal().is_none());
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("h");

    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.mode, Mode::Normal);
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());
}

#[test]
fn control_w_splits_an_only_terminal_without_starting_review() {
    for (suffix, status) in [('v', "vertical split"), ('s', "horizontal split")] {
        let mut session = Session::start("/bin/sh -c 'stty raw -echo; printf ready; cat -v'");
        assert!(session.settle(|app| terminal_text(app).contains("ready")));
        let terminal = session.app.active_terminal().unwrap();

        session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
        session.type_text(&suffix.to_string());

        assert_eq!(session.app.panes.len(), 2);
        assert_eq!(session.app.active_terminal(), None);
        assert_eq!(session.app.mode, Mode::Normal);
        assert_eq!(session.app.status, status);
        let child = session.app.terminals.get(terminal).unwrap();
        assert!(child.live());
        assert!(!child.reviewing());
        assert!(!terminal_text_by_id(&session.app, terminal).contains("^W"));
    }
}

#[test]
fn other_insert_keys_still_reach_the_child_unchanged() {
    let mut session = Session::start("/bin/sh -c 'stty raw -echo; printf ready; cat -v'");
    assert!(session.settle(|app| terminal_text(app).contains("ready")));
    for key in [
        KeyStroke::ctrl('c'),
        KeyStroke::plain(KeyCode::Escape),
        KeyStroke::ctrl('o'),
        KeyStroke::char(' '),
        KeyStroke::char('x'),
    ] {
        session.app.handle_key(key).unwrap();
    }
    assert_eq!(session.app.mode, Mode::Insert);
    assert!(session.settle(|app| {
        let text = terminal_text(app);
        text.contains("^C") && text.contains("^[") && text.contains("^O") && text.contains(" x")
    }));
}

/// Escape belongs to the child, which is the whole reason the leader exists.
#[test]
fn escape_is_sent_to_the_child_rather_than_leaving_insert_mode() {
    let mut session = Session::start("/bin/cat -v");
    session.press(KeyCode::Escape);
    session.press(KeyCode::Enter);
    assert_eq!(session.app.mode, Mode::Insert);
    // `cat -v` prints an escape as `^[`, so this is the child's own echo of
    // the byte rather than anything the editor decided.
    assert!(session.settle(|app| terminal_text(app).contains("^[")));
}

#[test]
fn a_terminal_pane_refuses_commands_that_would_edit_the_buffer_behind_it() {
    let mut session = Session::start("/bin/cat");
    let before = session.app.active_buffer().to_string();
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    session.type_text("d");
    assert_eq!(session.app.active_buffer().to_string(), before);
    assert!(
        session.app.status.contains("needs a buffer"),
        "{}",
        session.app.status
    );
}

#[test]
fn normal_mode_scrolls_the_scrollback_and_returns_to_the_live_screen() {
    let mut session =
        Session::start("/bin/sh -c 'for i in 1 2 3 4 5 6 7 8 9; do echo line $i; done; cat'");
    assert!(session.settle(|app| terminal_text(app).contains("line 9")));
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();

    // The pane is 10 rows of body at this size, so nine lines and a prompt fit
    // without scrollback; render taller output into a shorter pane instead.
    let screen = session.screen(60, 8);
    assert!(screen.contains("line 9"), "{screen}");

    session.type_text("gg");
    let top = session.screen(60, 8);
    assert!(top.contains("line 1"), "{top}");

    session.type_text("ge");
    let live = session.screen(60, 8);
    assert!(live.contains("line 9"), "{live}");
}

#[test]
fn terminal_goto_keys_move_the_review_caret() {
    let mut session = Session::start(
        r#"/bin/sh -c 'printf "one\r\n  two\r\n\r\nthree\r\nfour\r\nfive\r\nsix"; sleep 30'"#,
    );
    assert!(session.settle(|app| terminal_text(app).contains("six")));
    let _ = session.screen(60, 8);
    session.leave_input();
    let id = session.app.active_terminal().unwrap();

    session.type_text("gg");
    assert_eq!(session.app.terminals.get(id).unwrap().cursor_row(), 0);
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "o"
    );

    session.type_text("2gggs");
    assert_eq!(session.app.terminals.get(id).unwrap().cursor_row(), 1);
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "t"
    );

    session.type_text("gggp");
    assert_eq!(session.app.terminals.get(id).unwrap().cursor_row(), 3);
    session.type_text("gP");
    assert_eq!(session.app.terminals.get(id).unwrap().cursor_row(), 0);

    session.type_text("ge");
    assert_eq!(session.app.terminals.get(id).unwrap().cursor_row(), 6);
}

#[test]
fn terminal_normal_file_motions_move_the_review_caret() {
    let mut session = Session::start(
        r#"/bin/sh -c 'printf "alpha x omega\r\n"; for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo line$i; done; sleep 30'"#,
    );
    assert!(session.settle(|app| terminal_text(app).contains("line12")));
    let _ = session.screen(60, 12);
    session.leave_input();
    let id = session.app.active_terminal().unwrap();

    session.type_text("gg");
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('f'), Modifiers::CONTROL))
        .unwrap();
    let page_down = session.app.terminals.get(id).unwrap().cursor_row();
    assert!(page_down > 0);

    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('u'), Modifiers::CONTROL))
        .unwrap();
    let half_up = session.app.terminals.get(id).unwrap().cursor_row();
    assert!(half_up < page_down);
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('d'), Modifiers::CONTROL))
        .unwrap();
    assert!(session.app.terminals.get(id).unwrap().cursor_row() > half_up);
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('b'), Modifiers::CONTROL))
        .unwrap();
    assert_eq!(session.app.terminals.get(id).unwrap().cursor_row(), 0);

    session.type_text("ggfx");
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "x"
    );
    session.type_text("ggtx");
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        " "
    );
    session.type_text("ggWE");
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "a"
    );
    session.type_text("gggws");
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "o"
    );
}

#[test]
fn normal_mode_has_a_movable_caret_and_selects_and_copies_terminal_text() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "alpha\r\nbeta"; sleep 30'"#);
    assert!(session.settle(|app| terminal_text(app).contains("beta")));
    let clipboard = Arc::new(Mutex::new(String::new()));
    session
        .app
        .set_system_clipboard(Box::new(MemoryClipboard(Arc::clone(&clipboard))));

    session.leave_input();
    let id = session.app.active_terminal().unwrap();
    assert!(session.app.terminals.get(id).unwrap().reviewing());
    assert!(
        session
            .app
            .terminals
            .get(id)
            .unwrap()
            .view(10)
            .cursor
            .is_some()
    );

    session.press(KeyCode::Home);
    session.press(KeyCode::Char('v'));
    session.type_text("lll");
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "beta"
    );

    session.press(KeyCode::Escape);
    session.press(KeyCode::End);
    session.press(KeyCode::Char('v'));
    session.type_text("hhh");
    assert_eq!(review_selection(&mut session.app, id), "beta");

    session.type_text(" cy");
    assert_eq!(&*clipboard.lock().unwrap(), "beta");
}

#[test]
fn pointer_drag_selects_terminal_review_and_right_click_copies_it() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "alpha\r\nbeta"; sleep 30'"#);
    assert!(session.settle(|app| terminal_text(app).contains("beta")));
    let clipboard = Arc::new(Mutex::new(String::new()));
    session
        .app
        .set_system_clipboard(Box::new(MemoryClipboard(Arc::clone(&clipboard))));
    session.leave_input();
    let id = session.app.active_terminal().unwrap();

    let geometry = ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 60, 12));
    let view = session.app.prepare_view(geometry);
    let body = view.pane(session.app.active_pane).unwrap().body;
    let terminal = session
        .app
        .terminals
        .get(id)
        .unwrap()
        .view(usize::from(body.height));
    let review_row = terminal
        .rows
        .iter()
        .position(|cells| {
            cells
                .iter()
                .map(|cell| cell.character)
                .collect::<String>()
                .starts_with("alpha")
        })
        .expect("alpha is visible in the review");

    let pointer = |kind, column| PointerEvent {
        kind,
        column: body.x + column,
        row: body.y + u16::try_from(review_row).unwrap(),
        modifiers: Modifiers::NONE,
    };
    session
        .app
        .handle_pointer(
            pointer(PointerEventKind::Down(PointerButton::Left), 0),
            &view,
        )
        .unwrap();
    session
        .app
        .handle_pointer(
            pointer(PointerEventKind::Drag(PointerButton::Left), 4),
            &view,
        )
        .unwrap();
    session
        .app
        .handle_pointer(pointer(PointerEventKind::Up(PointerButton::Left), 4), &view)
        .unwrap();

    assert_eq!(session.app.mode, Mode::Select);
    assert_eq!(review_selection(&mut session.app, id), "alpha");

    session
        .app
        .handle_pointer(
            pointer(PointerEventKind::Down(PointerButton::Left), 4),
            &view,
        )
        .unwrap();
    session
        .app
        .handle_pointer(
            pointer(PointerEventKind::Drag(PointerButton::Left), 0),
            &view,
        )
        .unwrap();
    session
        .app
        .handle_pointer(pointer(PointerEventKind::Up(PointerButton::Left), 0), &view)
        .unwrap();
    assert_eq!(review_selection(&mut session.app, id), "alpha");

    session
        .app
        .handle_pointer(
            pointer(PointerEventKind::Down(PointerButton::Right), 2),
            &view,
        )
        .unwrap();
    assert_eq!(&*clipboard.lock().unwrap(), "alpha");
    assert_eq!(review_selection(&mut session.app, id), "alpha");
    assert_eq!(
        session.app.snapshot(&view).status.interaction_line,
        "right mouse click (yanked to system clipboard)"
    );
}

#[test]
fn percent_selects_all_terminal_review_text() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "alpha\r\nbeta"; sleep 30'"#);
    assert!(session.settle(|app| terminal_text(app).contains("beta")));
    session.leave_input();
    let id = session.app.active_terminal().unwrap();

    session.press(KeyCode::Char('%'));

    assert_eq!(session.app.mode, Mode::Select);
    assert_eq!(review_selection(&mut session.app, id), "alpha\nbeta");
}

#[test]
fn terminal_p_and_uppercase_p_send_the_runyte_register_and_enter_input() {
    let mut session = Session::start(
        r#"/bin/sh -c 'printf "copyme\r\n"; IFS= read -r first; printf "first:%s\r\n" "$first"; IFS= read -r second; printf "second:%s\r\n" "$second"; sleep 30'"#,
    );
    assert!(session.settle(|app| terminal_text(app).contains("copyme")));
    let clipboard = Arc::new(Mutex::new("system-only\n".to_owned()));
    session
        .app
        .set_system_clipboard(Box::new(MemoryClipboard(clipboard)));

    session.leave_input();
    session.type_text("ggvlllllly");
    session.press(KeyCode::Char('p'));
    assert_eq!(session.app.mode, Mode::Insert);
    session.press(KeyCode::Enter);
    assert!(session.settle(|app| terminal_text(app).contains("first:copyme")));

    session.leave_input();
    session.press(KeyCode::Char('P'));
    assert_eq!(session.app.mode, Mode::Insert);
    session.press(KeyCode::Enter);
    assert!(session.settle(|app| terminal_text(app).contains("second:copyme")));
}

/// Escape ends a review selection whichever key began it. `x` hands the mode
/// back before the terminal ever sees the command, so a fix that only looked
/// at SELECT left the line selected and the caret spanning it.
#[test]
fn escape_cancels_a_terminal_review_selection_from_both_v_and_x() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "alpha\r\nbeta\r\ngamma"; sleep 30'"#);
    assert!(session.settle(|app| terminal_text(app).contains("gamma")));
    session.leave_input();
    let id = session.app.active_terminal().unwrap();

    session.type_text("gg");
    session.press(KeyCode::Char('x'));
    assert_eq!(session.app.mode, Mode::Select);
    assert_eq!(review_selection(&mut session.app, id), "alpha");

    session.press(KeyCode::Escape);
    assert_eq!(session.app.mode, Mode::Normal);
    assert_eq!(review_selection(&mut session.app, id).chars().count(), 1);

    session.type_text("ggvll");
    assert_eq!(session.app.mode, Mode::Select);
    assert_eq!(review_selection(&mut session.app, id), "alp");

    session.press(KeyCode::Escape);
    assert_eq!(session.app.mode, Mode::Normal);
    assert_eq!(review_selection(&mut session.app, id).chars().count(), 1);
}

/// A terminal has no undo history, so `u` asks the child for the paste back
/// with one delete per character sent — and only while that paste is still the
/// last thing the child received.
#[test]
fn terminal_u_asks_the_child_to_erase_the_last_paste() {
    let mut session = Session::start("/bin/cat");
    let clipboard = Arc::new(Mutex::new("erase-me".to_owned()));
    session
        .app
        .set_system_clipboard(Box::new(MemoryClipboard(Arc::clone(&clipboard))));

    session.leave_input();
    session.press(KeyCode::Char('u'));
    assert!(
        session.app.status.contains("nothing Runyte sent"),
        "{}",
        session.app.status
    );

    session.type_text(" cp");
    assert!(session.settle(|app| terminal_text(app).contains("erase-me")));

    session.press(KeyCode::Char('u'));
    assert!(
        session.settle(|app| !terminal_text(app).contains("erase-me")),
        "{:?}",
        terminal_text(&session.app)
    );
    assert!(
        session.app.status.contains("erase 8 pasted characters"),
        "{}",
        session.app.status
    );

    // One undo per paste: a second must not eat what the paste was typed into.
    session.press(KeyCode::Char('u'));
    assert!(
        session.app.status.contains("nothing Runyte sent"),
        "{}",
        session.app.status
    );
}

/// What the child has already run is the child's. Deletes would erase the next
/// prompt's line rather than the paste, so `u` says so instead of sending them.
#[test]
fn terminal_u_refuses_a_paste_the_child_has_already_run() {
    let mut session = Session::start("/bin/cat");
    let clipboard = Arc::new(Mutex::new("already-run\n".to_owned()));
    session
        .app
        .set_system_clipboard(Box::new(MemoryClipboard(Arc::clone(&clipboard))));

    session.leave_input();
    session.type_text(" cp");
    // `cat` writing the line back is the proof that it ran rather than sat in
    // the line editor: the echo alone would have appeared either way.
    assert!(session.settle(|app| terminal_text(app).matches("already-run").count() >= 2));

    session.press(KeyCode::Char('u'));
    assert!(
        session.app.status.contains("already run"),
        "{}",
        session.app.status
    );
    assert!(terminal_text(&session.app).contains("already-run"));
}

#[test]
fn terminal_review_line_and_multi_selection_commands_copy_together() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "alpha\r\nbeta\r\ngamma"; sleep 30'"#);
    assert!(session.settle(|app| terminal_text(app).contains("gamma")));
    let clipboard = Arc::new(Mutex::new(String::new()));
    session
        .app
        .set_system_clipboard(Box::new(MemoryClipboard(Arc::clone(&clipboard))));

    session.leave_input();
    let id = session.app.active_terminal().unwrap();
    session.press(KeyCode::Home);
    session.type_text("k");

    session.press(KeyCode::Char('C'));
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "b\ng"
    );
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('C'), Modifiers::ALT))
        .unwrap();
    assert_eq!(
        session
            .app
            .terminals
            .get(id)
            .unwrap()
            .review_selection_count(),
        3
    );

    session.press(KeyCode::Char('X'));
    assert_eq!(session.app.mode, Mode::Select);
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "alpha\nbeta\ngamma"
    );
    session.press(KeyCode::Char('x'));
    session.type_text(" cy");

    assert_eq!(&*clipboard.lock().unwrap(), "alpha\nbeta\ngamma\n");
}

#[test]
fn terminal_review_comma_and_semicolon_manage_copied_selections() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "alpha\r\nbeta\r\ngamma"; sleep 30'"#);
    assert!(session.settle(|app| terminal_text(app).contains("gamma")));
    session.leave_input();
    let id = session.app.active_terminal().unwrap();
    session.press(KeyCode::Home);
    session.type_text("k");

    session.press(KeyCode::Char('C'));
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('C'), Modifiers::ALT))
        .unwrap();
    assert_eq!(
        session
            .app
            .terminals
            .get(id)
            .unwrap()
            .review_selection_count(),
        3
    );

    session.type_text(",");
    assert_eq!(
        session
            .app
            .terminals
            .get(id)
            .unwrap()
            .review_selection_count(),
        1
    );
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "b"
    );

    session.press(KeyCode::Char('C'));
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('C'), Modifiers::ALT))
        .unwrap();
    session.type_text("vll");
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "alp\nbet\ngam"
    );

    session.type_text(";");
    assert_eq!(session.app.mode, Mode::Normal);
    assert_eq!(
        session
            .app
            .terminals
            .get(id)
            .unwrap()
            .review_selection_count(),
        3
    );
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "p\nt\nm"
    );
}

#[test]
fn the_pane_is_named_by_the_title_the_child_sets() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "\033]0;agent\007"; cat'"#);
    assert!(session.settle(|app| {
        app.active_terminal()
            .and_then(|id| app.terminals.get(id))
            .is_some_and(|session| session.name() == "agent")
    }));
    let insert = session.screen(60, 12);
    assert!(insert.contains("[terminal] agent [insert]"), "{insert}");

    // NORMAL is the unmarked state: leaving input drops the marker rather
    // than replacing it, so the title only ever answers whether typing
    // reaches the child.
    session.leave_input();
    let normal = session.screen(60, 12);
    assert!(normal.contains("[terminal] agent"), "{normal}");
    assert!(!normal.contains("[insert]"), "{normal}");
    assert!(!normal.contains("[normal]"), "{normal}");
}

#[test]
fn exiting_the_last_terminal_reveals_its_buffer_without_quitting_runyte() {
    let mut session = Session::start("/bin/sh");
    let underlying = session.app.active().buffer;
    let terminal = session.app.active_terminal().unwrap();
    session.type_text("exit");
    session.press(KeyCode::Enter);

    assert!(session.settle(|app| {
        app.terminals
            .get(terminal)
            .is_some_and(|terminal| terminal.exit_code().is_some())
    }));
    assert_eq!(session.app.terminals.len(), 1);
    assert_eq!(session.app.panes.len(), 1);
    assert_eq!(session.app.active_terminal(), None);
    assert_eq!(session.app.active().buffer, underlying);
    assert!(!session.app.should_quit);

    session.type_text(" tt");
    assert!(
        session
            .app
            .overlay_snapshots()
            .iter()
            .any(|overlay| overlay.kind == OverlayKind::ResultList)
    );
}

#[test]
fn exiting_a_terminal_preserves_its_pane_when_another_pane_exists() {
    let mut session = Session::start("/bin/sh");
    let terminal = session.app.active_terminal().unwrap();
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("v");
    assert_eq!(session.app.panes.len(), 2);
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('w'), Modifiers::CONTROL))
        .unwrap();
    session.press(KeyCode::Char('w'));
    assert!(session.app.active_terminal().is_some());
    assert_eq!(session.app.mode, Mode::Insert);
    session.type_text("exit");
    session.press(KeyCode::Enter);

    assert!(session.settle(|app| {
        app.terminals
            .get(terminal)
            .is_some_and(|terminal| terminal.exit_code().is_some())
    }));
    assert_eq!(session.app.terminals.len(), 1);
    assert_eq!(session.app.panes.len(), 2);
    assert_eq!(session.app.active_terminal(), None);
    assert!(!session.app.should_quit);
}

#[test]
fn closing_a_document_pane_preserves_terminal_review() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();
    session.leave_input();
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());

    session.colon("vsplit");
    assert!(session.app.active_terminal().is_none());
    session.colon("quit!");

    assert_eq!(session.app.active_terminal(), Some(terminal));
    assert_eq!(session.app.mode, Mode::Normal);
    assert!(session.app.terminals.get(terminal).unwrap().reviewing());
}

#[test]
fn leaving_a_terminal_shows_the_buffer_again_without_ending_the_child() {
    let mut session = Session::start("/bin/cat");
    let id = session.app.active_terminal().unwrap();
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    session.type_text(" tq");
    assert!(session.app.active_terminal().is_none());
    assert!(
        session
            .app
            .terminals
            .get(id)
            .is_some_and(|session| session.live()),
        "leaving the view ended the child"
    );

    // The list is how it is reached again.
    session.type_text(" tt");
    session.press(KeyCode::Enter);
    assert_eq!(session.app.active_terminal(), Some(id));
}

#[test]
fn buffer_picker_reveals_the_buffer_under_the_terminal_without_ending_the_child() {
    let directory = std::env::temp_dir().join(format!(
        "runyte-terminal-buffer-picker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let path = directory.join("under-terminal.txt");
    std::fs::write(&path, "the selected buffer\n").unwrap();

    let mut session = Session::start_with_file(Config::default(), "/bin/cat", Some(path.clone()));
    let id = session.app.active_terminal().unwrap();
    let underlying = session.app.active().buffer;

    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    assert_eq!(session.app.mode, Mode::Normal);
    session.type_text(" bb");
    session.type_text("under-terminal");
    session.press(KeyCode::Enter);

    assert_eq!(session.app.active_terminal(), None);
    assert_eq!(session.app.active().buffer, underlying);
    assert_eq!(
        session.app.active_buffer().path.as_deref(),
        Some(path.as_path())
    );
    assert!(
        session
            .app
            .terminals
            .get(id)
            .is_some_and(|terminal| terminal.live())
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn buffer_picker_retargets_a_terminal_pane_to_a_different_open_buffer() {
    let directory = std::env::temp_dir().join(format!(
        "runyte-terminal-buffer-picker-other-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let first = directory.join("first-buffer.txt");
    let second = directory.join("second-buffer.txt");
    std::fs::write(&first, "first\n").unwrap();
    std::fs::write(&second, "second\n").unwrap();

    let mut app = App::new(Config::default(), Some(first.clone())).unwrap();
    type_colon(&mut app, &format!("open {}", second.display()));
    let mut session = Session::start_from_app(app, "/bin/cat");
    let id = session.app.active_terminal().unwrap();
    let covered = session.app.active().buffer;

    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    session.type_text(" bb");
    session.type_text("first-buffer");
    session.press(KeyCode::Enter);

    assert_eq!(session.app.active_terminal(), None);
    assert_ne!(session.app.active().buffer, covered);
    assert_eq!(
        session.app.active_buffer().path.as_deref(),
        Some(first.as_path())
    );
    assert_eq!(session.app.mode, Mode::Normal);
    assert!(
        session
            .app
            .terminals
            .get(id)
            .is_some_and(|terminal| terminal.live())
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn showing_a_reviewed_terminal_preserves_review() {
    let mut session = Session::start("/bin/cat");
    let id = session.app.active_terminal().unwrap();

    session.leave_input();
    assert!(session.app.terminals.get(id).unwrap().reviewing());
    session.type_text(" tq");
    session.colon("vsplit");

    // Every existing-session activation reaches `show_terminal`: the manager,
    // resource finder, and `:terminal-show` differ only in how they choose the
    // id. Showing the session is not itself a request to resume child input.
    session.type_text(" tt");
    session.press(KeyCode::Enter);
    assert_eq!(session.app.active_terminal(), Some(id));
    assert_eq!(session.app.mode, Mode::Normal);
    assert!(session.app.terminals.get(id).unwrap().reviewing());

    render(&mut session.app, 60, 12);
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.type_text("h");
    assert!(session.app.active_terminal().is_none());
    assert_eq!(session.app.mode, Mode::Normal);
    assert!(session.app.terminals.get(id).unwrap().reviewing());
}

#[test]
fn close_refuses_a_terminal_and_quit_only_removes_its_pane() {
    let mut session = Session::start("/bin/cat");
    let id = session.app.active_terminal().unwrap();
    session.colon("close!");
    assert_eq!(session.app.active_terminal(), Some(id));
    assert!(session.app.terminals.get(id).unwrap().live());
    assert!(session.app.status.contains("not a buffer"));

    session.colon("vsplit");
    session.app.handle_key(KeyStroke::ctrl('w')).unwrap();
    session.press(KeyCode::Char('w'));
    assert_eq!(session.app.active_terminal(), Some(id));
    session.colon("quit!");
    assert_eq!(session.app.panes.len(), 1);
    assert!(session.app.terminals.get(id).unwrap().live());
    assert!(
        session
            .app
            .panes
            .values()
            .all(|pane| pane.terminal != Some(id))
    );
}

#[test]
fn closing_a_terminal_ends_its_child_and_forgets_it() {
    let mut session = Session::start("/bin/cat");
    let id = session.app.active_terminal().unwrap();
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    session.type_text(" tt");
    session.press(KeyCode::Tab);
    session.press(KeyCode::Down);
    session.press(KeyCode::Down);
    session.press(KeyCode::Enter);
    assert!(session.app.active_terminal().is_none());
    assert!(session.app.terminals.get(id).is_none());
}

#[test]
fn renaming_a_listed_terminal_leaves_every_pane_showing_what_it_showed() {
    let mut session = Session::start("/bin/cat");
    let first = session.app.active_terminal().unwrap();
    session.colon("terminal /bin/cat");
    let second = session.app.active_terminal().unwrap();
    assert_ne!(first, second);

    session.leave_input();
    session.type_text(" tt");
    // The first terminal is the row the list opens on, and it is not the one
    // the pane is showing.
    session.press(KeyCode::Tab);
    session.press(KeyCode::Down);
    session.press(KeyCode::Enter);
    session.type_text("alpha");
    session.press(KeyCode::Enter);

    assert_eq!(
        session.app.terminals.get(first).unwrap().user_name(),
        Some("alpha")
    );
    // Naming a terminal is not a way of reaching it: the pane still shows the
    // terminal it showed, and the list is still what the person is looking at.
    assert_eq!(session.app.active_terminal(), Some(second));
    assert_eq!(
        session
            .app
            .list
            .as_ref()
            .map(|picker| picker.title.as_str()),
        Some("Terminals")
    );
}

#[test]
fn abandoning_a_listed_terminal_rename_changes_nothing_and_returns_to_the_list() {
    let mut session = Session::start("/bin/cat");
    let first = session.app.active_terminal().unwrap();
    session.colon("terminal /bin/cat");
    let second = session.app.active_terminal().unwrap();

    session.leave_input();
    session.type_text(" tt");
    session.press(KeyCode::Tab);
    session.press(KeyCode::Down);
    session.press(KeyCode::Enter);
    session.type_text("alpha");
    session.press(KeyCode::Escape);

    assert_eq!(session.app.terminals.get(first).unwrap().user_name(), None);
    assert_eq!(session.app.active_terminal(), Some(second));
    assert_eq!(
        session
            .app
            .list
            .as_ref()
            .map(|picker| picker.title.as_str()),
        Some("Terminals")
    );
}

#[test]
fn copying_a_terminals_output_opens_it_as_an_ordinary_buffer() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "copied text\r\n"; cat'"#);
    assert!(session.settle(|app| terminal_text(app).contains("copied text")));
    session.colon("terminal-output");
    // The pane now shows a document, so the terminal is no longer what it
    // draws, and the document is real searchable text.
    assert!(session.app.active_terminal().is_none());
    assert!(
        session
            .app
            .active_buffer()
            .to_string()
            .contains("copied text")
    );
}

#[test]
fn terminal_review_repeats_regex_matches_with_n_uppercase_n_and_parentheses() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "one two one"; cat'"#);
    assert!(session.settle(|app| terminal_text(app).contains("one two one")));
    session.leave_input();
    session.press(KeyCode::Char('S'));
    session.type_text("one|two");
    session.press(KeyCode::Enter);
    let id = session.app.active_terminal().unwrap();
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "one"
    );

    session.press(KeyCode::Char('n'));
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "two"
    );
    session.press(KeyCode::Char(')'));
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "one"
    );
    session.press(KeyCode::Char('N'));
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "two"
    );
    session.press(KeyCode::Char('('));
    assert_eq!(
        session
            .app
            .terminals
            .get_mut(id)
            .unwrap()
            .review_selection_text(),
        "one"
    );
    assert!(!session.app.status.contains("needs a buffer"));
}

#[test]
fn terminal_output_remains_jumpable_after_its_pane_returns_to_the_terminal() {
    let mut session = Session::start(r#"/bin/sh -c 'printf "jumpable\r\n"; cat'"#);
    assert!(session.settle(|app| terminal_text(app).contains("jumpable")));
    let id = session.app.active_terminal().unwrap();
    session.colon("terminal-output");
    let output = session.app.active().buffer;

    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('o'), Modifiers::CONTROL))
        .unwrap();
    assert_eq!(session.app.active_terminal(), Some(id));
    assert!(session.app.buffers[output].is_special());
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('i'), Modifiers::CONTROL))
        .unwrap();
    assert_eq!(session.app.active_terminal(), None);
    assert_eq!(session.app.active().buffer, output);
}

#[test]
fn terminal_manager_tab_draws_actions_and_space_t_r_renames() {
    let mut session = Session::start("/bin/cat");
    let id = session.app.active_terminal().unwrap();
    session.leave_input();
    session.type_text(" tt");
    let overlays = session.app.overlay_snapshots();
    let manager = overlays
        .iter()
        .find(|overlay| overlay.kind == OverlayKind::ResultList)
        .expect("terminal manager overlay");
    assert!(
        manager
            .rows
            .iter()
            .any(|row| row.label.starts_with("[terminal] "))
    );
    session.press(KeyCode::Tab);
    let screen = session.screen(60, 12);
    assert!(screen.contains("Terminal actions"), "{screen}");
    assert!(screen.contains("Rename"), "{screen}");
    assert!(screen.contains("Create"), "{screen}");

    session.press(KeyCode::Escape);
    session.press(KeyCode::Escape);
    assert_eq!(session.app.active_terminal(), Some(id));
    session.type_text(" tr");
    assert_eq!(session.app.mode, Mode::Command);
    session.type_text("agent");
    session.press(KeyCode::Enter);
    assert_eq!(session.app.terminals.get(id).unwrap().name(), "agent");
}

#[test]
fn showing_a_visible_terminal_in_another_pane_moves_its_single_view() {
    let mut session = Session::start("/bin/cat");
    let id = session.app.active_terminal().unwrap();
    let source = session.app.active_pane;
    session.colon("vsplit");
    let target = session.app.active_pane;
    assert_ne!(source, target);
    assert_eq!(session.app.terminal_of_pane(source), Some(id));
    assert_eq!(session.app.terminal_of_pane(target), None);

    session.type_text(" tt");
    session.press(KeyCode::Enter);

    assert_eq!(session.app.active_terminal(), Some(id));
    assert_eq!(session.app.terminal_of_pane(source), None);
    assert_eq!(session.app.terminal_of_pane(target), Some(id));
    assert_eq!(
        session
            .app
            .panes
            .keys()
            .filter(|pane| session.app.terminal_of_pane(**pane) == Some(id))
            .count(),
        1
    );

    // An odd height gives the two split panes different body heights. With a
    // duplicated view, every frame would shrink and regrow the same grid and
    // manufacture scrollback. A moved view has one stable authoritative size.
    session.screen(60, 13);
    let lines = session.app.terminals.get(id).unwrap().line_count();
    for _ in 0..3 {
        session.screen(60, 13);
    }
    assert_eq!(session.app.terminals.get(id).unwrap().line_count(), lines);
}

#[test]
fn detached_output_for_a_hidden_terminal_stays_unread() {
    let mut session = Session::start("/bin/cat");
    let id = session.app.active_terminal().unwrap();
    session.leave_input();
    session.type_text(" tq");
    session.app.apply_terminal_output_observed(
        TerminalOutput::Bytes {
            id,
            bytes: b"detached output".to_vec(),
        },
        false,
    );
    assert!(session.app.terminals.get(id).unwrap().unread_activity());

    session.type_text(" tt");
    let overlays = session.app.overlay_snapshots();
    let manager = overlays
        .iter()
        .find(|overlay| overlay.kind == OverlayKind::ResultList)
        .expect("terminal manager overlay");
    assert!(manager.rows.iter().any(|row| row.detail.contains("unread")));
}

#[test]
fn a_selection_composed_in_a_buffer_can_be_sent_to_a_terminal() {
    let mut session = Session::start("/bin/cat");
    // Leave the terminal on screen in this pane and compose in a split.
    session.colon("vsplit");
    assert!(session.app.active_terminal().is_none());
    session.type_text("i");
    session.type_text("composed prompt");
    session
        .app
        .handle_key(KeyStroke::plain(KeyCode::Escape))
        .unwrap();
    session.colon("terminal-send");
    session.press(KeyCode::Enter);
    assert!(session.settle(|app| {
        app.terminals
            .iter()
            .any(|session| session.plain_text().contains("composed prompt"))
    }));
}

/// A child writing without pause must not be able to hold the editor. The
/// queue is bounded, so it cannot grow without limit either; between them a
/// `yes` in a pane costs one bounded batch per frame and nothing else.
#[test]
fn a_child_that_never_stops_writing_cannot_starve_the_editor() {
    let mut session = Session::start("/bin/sh -c 'while true; do echo flood; done'");
    // Let the child fill the queue and keep it full.
    std::thread::sleep(Duration::from_millis(200));

    for _ in 0..5 {
        let taken = terminal::drain(&mut session.output, |output| {
            session.app.apply_terminal_output(output);
        });
        assert!(
            taken <= OUTPUT_QUEUE,
            "one drain took {taken}, past the bound"
        );
    }

    // The editor is still reachable, and the session is still bounded.
    session
        .app
        .handle_key(KeyStroke::new(KeyCode::Char('\\'), Modifiers::CONTROL))
        .unwrap();
    assert_eq!(session.app.mode, Mode::Normal);
    let id = session.app.active_terminal().unwrap();
    let lines = session
        .app
        .terminals
        .get(id)
        .unwrap()
        .plain_text()
        .lines()
        .count();
    assert!(lines <= 5_100, "scrollback grew to {lines} lines");

    // Keep the queue saturated through shutdown. Dropping the terminal owner
    // must unregister and wake its blocked PTY reader before reaping the
    // deliberately endless child. Bound this separately so a regression
    // fails here instead of consuming the CI job's complete timeout.
    let terminals = std::mem::take(&mut session.app.terminals);
    let (finished, completion) = mpsc::sync_channel(1);
    let shutdown = std::thread::spawn(move || {
        drop(terminals);
        let _ = finished.send(());
    });
    completion
        .recv_timeout(Duration::from_secs(5))
        .expect("saturated terminal shutdown timed out");
    shutdown.join().unwrap();
}

#[test]
fn quitting_refuses_a_running_terminal_even_when_forced() {
    let mut session = Session::start("/bin/cat");
    session.colon("quit");
    assert!(!session.app.should_quit);
    assert!(
        session.app.status.contains("still running"),
        "{}",
        session.app.status
    );
    session.colon("quit!");
    assert!(!session.app.should_quit);
    assert!(session.app.terminals.iter().all(|terminal| terminal.live()));
}

#[test]
fn persistent_quit_refuses_a_running_terminal_even_when_forced() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();
    session.app.enable_persistent_session();

    session.colon("quit");
    assert!(!session.app.should_quit);
    assert!(session.app.status.contains("still running"));
    session.colon("quit!");
    assert!(!session.app.should_quit);
    assert!(session.app.terminals.get(terminal).unwrap().live());
}

#[test]
fn persistent_detach_leaves_a_terminal_running() {
    let mut session = Session::start("/bin/cat");
    let terminal = session.app.active_terminal().unwrap();
    session.app.enable_persistent_session();

    session.colon("detach");

    assert!(session.app.should_quit);
    assert!(session.app.terminals.get(terminal).unwrap().live());
}
