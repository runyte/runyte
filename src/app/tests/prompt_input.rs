// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;

const REJECTED: &str = "Control characters are not allowed; nothing was inserted";

fn fixture() -> (TestRuntimeRoot, App) {
    let root = TestRuntimeRoot::new("prompt-paste").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let app = App::new_in_isolated_project(
        &project,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    (root, app)
}

fn paste(app: &mut App, text: &str) {
    app.handle_input(InputEvent::Text(text.into())).unwrap();
}

fn visible_error(app: &App) -> bool {
    app.overlay_snapshots().iter().any(|overlay| {
        overlay
            .message
            .as_deref()
            .is_some_and(|message| message.contains(REJECTED))
    })
}

#[test]
fn rejected_paste_feedback_is_drawn_in_standalone_and_attached_frontends() {
    use ratatui::{Terminal, backend::TestBackend};

    for keys in [":", ":open absent", "s", " f", " bb"] {
        for (attached, width, height) in [
            (false, 120, 40),
            (true, 120, 40),
            (false, 60, 20),
            (true, 60, 20),
        ] {
            let (_root, mut app) = fixture();
            if keys.starts_with(' ') {
                for index in 0..60 {
                    app.host_open_file(
                        app.working_directory.join(format!("entry-{index:02}")),
                        false,
                    )
                    .unwrap();
                }
            }
            for character in keys.chars() {
                press(&mut app, character);
            }
            paste(&mut app, "hidden\nsuffix");
            let mut host = crate::workspace::WorkspaceHost::new(app);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for rejected in [true, false] {
                terminal
                    .draw(|frame| {
                        let snapshot = host.prepare_frame(crate::ui::frame_geometry(frame.area()));
                        if attached {
                            crate::ui::render_host_frame_exact_colors_for_test(frame, &snapshot);
                        } else {
                            crate::ui::render_exact_colors_for_test(
                                frame,
                                host.app(),
                                &snapshot.editor,
                                &crate::key_hints::KeyHintState::default(),
                            );
                        }
                    })
                    .unwrap();
                let rows = terminal
                    .backend()
                    .buffer()
                    .content
                    .chunks(usize::from(width))
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>();
                let screen = rows.join("\n");
                // Reassemble the message within its column across soft wraps.
                let rendered_rejection = rows
                    .iter()
                    .enumerate()
                    .find_map(|(row, line)| {
                        let column = line
                            .split('│')
                            .position(|text| text.contains("Control characters"))?;
                        Some(
                            rows[row..]
                                .iter()
                                .filter_map(|line| line.split('│').nth(column))
                                .flat_map(str::split_whitespace)
                                .collect::<Vec<_>>()
                                .join(" ")
                                .starts_with(REJECTED),
                        )
                    })
                    .unwrap_or(false);
                assert_eq!(
                    rendered_rejection, rejected,
                    "{keys:?}, attached={attached}: {screen}"
                );
                assert!(!screen.contains("suffix"));
                if keys == ":open absent" {
                    assert!(screen.contains("No matching paths"));
                }
                if keys == "s" && rejected {
                    // The feedback panel ends just above the global status and
                    // editable interaction lines, even in a narrow terminal.
                    let buffer = terminal.backend().buffer();
                    assert_eq!(buffer[(0, height - 3)].symbol(), "└");
                }
                paste(host.app_mut(), "alpha");
            }
        }
    }
}

#[test]
fn command_paste_cannot_open_a_hidden_newline_suffix_and_clean_paste_recovers() {
    let (_root, mut app) = fixture();
    press(&mut app, ':');
    paste(&mut app, "open ");
    let before = app.buffers.len();
    paste(&mut app, "alpha\nbeta");
    assert_eq!(app.command, "open ");
    assert_eq!(app.command_cursor, 5);
    assert!(visible_error(&app));
    assert!(
        app.notifications
            .entries()
            .iter()
            .all(|entry| !entry.body.contains("beta"))
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.buffers.len(), before);
    paste(&mut app, "alpha");
    assert!(app.prompt_input_error.is_none());
    assert!(!visible_error(&app));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(app.working_directory.join("alpha").as_path())
    );
}

#[test]
fn single_line_prompt_kinds_reject_controls_preserve_cursor_and_allow_literal_escapes() {
    for kind in [
        PromptKind::Command,
        PromptKind::Search(SearchMode::Regex),
        PromptKind::Rename,
        PromptKind::FinderPath,
        PromptKind::ExternalProgram,
        PromptKind::JoinDelimiter,
    ] {
        let (_root, mut app) = fixture();
        app.open_prompt_with_value(kind, "éabc".into());
        app.command_cursor = 2;
        app.command_selection = 1;
        for text in ["left\nright", "value\r\n", "a\tb", "a\0b", "a\u{85}b"] {
            paste(&mut app, text);
            assert_eq!(app.command, "éabc");
            assert_eq!(app.command_cursor, 2);
            assert_eq!(app.command_selection, 1);
            assert!(visible_error(&app), "{kind:?}");
        }
        app.handle_key(KeyStroke::char('\u{85}')).unwrap();
        assert_eq!(app.command, "éabc");
        assert!(visible_error(&app));
        paste(&mut app, r"\n");
        assert_eq!(app.command, "éa\\nbc");
        assert_eq!(app.command_cursor, 4);
        assert!(!visible_error(&app));
    }
}

#[test]
fn prefilled_command_and_search_controls_are_rejected_on_submit() {
    for kind in [PromptKind::Command, PromptKind::Search(SearchMode::Regex)] {
        let (_root, mut app) = fixture();
        app.open_prompt_with_value(kind, "open alpha\nbeta".into());
        let before = app.buffers.len();
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.buffers.len(), before);
        assert_eq!(
            app.prompt_input_error,
            Some("Control characters are not allowed; input was not submitted")
        );
        app.close_prompt();
        assert!(app.prompt_input_error.is_none());
    }
}

#[test]
fn finder_and_list_pastes_preserve_query_selection_and_buffer() {
    let (_root, mut app) = fixture();
    fs::write(app.working_directory.join("alpha"), "text\n").unwrap();
    app.open_project_picker().unwrap();
    paste(&mut app, "alpha");
    app.picker.as_mut().unwrap().query_cursor = 2;
    let selected = app.picker.as_ref().unwrap().selected;
    paste(&mut app, "hidden\n");
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.query, "alpha");
    assert_eq!(picker.query_cursor, 2);
    assert_eq!(picker.selected, selected);
    assert!(visible_error(&app));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.list = Some(ListPicker::fuzzy(
        "Choices",
        vec![
            crate::picker::PickerItem::new("alpha", "", 0),
            crate::picker::PickerItem::new("beta", "", 1),
        ],
    ));
    paste(&mut app, "a");
    app.list.as_mut().unwrap().selected = 1;
    paste(&mut app, "hidden\t");
    assert_eq!(app.list.as_ref().unwrap().filter, "a");
    assert_eq!(app.list.as_ref().unwrap().selected, 1);
    assert!(visible_error(&app));
    assert_eq!(app.active_buffer().to_string(), "");
}

#[test]
fn session_directory_paste_preserves_completion_state() {
    let (_root, mut app) = fixture();
    let child = app.working_directory.join("child");
    fs::create_dir_all(&child).unwrap();
    app.enable_persistent_session();
    app.open_session_directory_chooser();
    paste(&mut app, "chi");
    let mut before = app.overlay_snapshots();
    paste(&mut app, "\nelsewhere");
    assert!(visible_error(&app));
    let mut after = app.overlay_snapshots();
    for overlay in before.iter_mut().chain(after.iter_mut()) {
        overlay.message = None;
    }
    assert_eq!(before, after);
    paste(&mut app, "ld");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        switch_target_path(&app.take_workspace_switch().unwrap()),
        child
    );
}

#[test]
fn typed_git_confirmations_reject_controls_without_changing_authorization() {
    use crate::git::{BranchDeletionPlan, DeletionAuthorization, Repository, WorktreeRemovalPlan};
    for kind in ["switch", "delete", "worktree"] {
        let (_root, mut app) = fixture();
        match kind {
            "switch" => {
                app.git_branch_switch = Some(BranchSwitchConfirmation {
                    repository: Repository::new(&app.working_directory),
                    action: BranchSwitch::Checkout {
                        branch: "feature".into(),
                    },
                    input: "feat".into(),
                    cursor: 2,
                })
            }
            "delete" => {
                app.git_branch_deletion = Some(BranchDeletionConfirmation {
                    plan: BranchDeletionPlan {
                        branch: "feature".into(),
                        tip: "a".repeat(40),
                        upstream: None,
                        retaining_branches: vec![],
                        required_authorization: DeletionAuthorization::Typed,
                    },
                    cascade: None,
                    input: "feat".into(),
                    cursor: 2,
                })
            }
            _ => {
                app.git_worktree_removal = Some(WorktreeRemovalConfirmation {
                    plan: WorktreeRemovalPlan {
                        path: app.working_directory.join("feature"),
                        head: None,
                        branch: Some("feature".into()),
                        upstream: None,
                        detached_retained: false,
                        required_authorization: DeletionAuthorization::Typed,
                    },
                    session: None,
                    #[cfg(windows)]
                    reviewed_live: None,
                    input: "feat".into(),
                    cursor: 2,
                })
            }
        }
        let before = (
            app.git_branch_switch.clone(),
            app.git_branch_deletion.clone(),
            app.git_worktree_removal.clone(),
        );
        let revision = app.confirmation_revision;
        paste(&mut app, "hidden\n");
        assert_eq!(
            before,
            (
                app.git_branch_switch.clone(),
                app.git_branch_deletion.clone(),
                app.git_worktree_removal.clone()
            )
        );
        assert_eq!(revision, app.confirmation_revision);
        assert!(visible_error(&app));
        assert!(app.overlay_snapshots().iter().any(|overlay| {
            overlay
                .message
                .as_deref()
                .is_some_and(|m| m.contains("feature"))
        }));
        paste(&mut app, "x");
        assert!(!visible_error(&app));
    }
}

#[test]
fn multiline_buffer_paste_remains_one_literal_edit() {
    let (_root, mut app) = fixture();
    press(&mut app, 'i');
    paste(&mut app, "alpha\nbeta\tend");
    assert_eq!(app.active_buffer().to_string(), "alpha\nbeta\tend");
    assert!(app.prompt_input_error.is_none());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'u');
    assert_eq!(app.active_buffer().to_string(), "");
}

fn configured_prompt(source: &str, kind: PromptKind, text: &str) -> App {
    let mut config = Config::default();
    config.merge_keys_source(source).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.open_prompt_with_value(kind, text.into());
    app
}
fn prompt_key(app: &mut App, key: &str) {
    app.handle_key(KeyStroke::parse(key).unwrap()).unwrap();
}

#[test]
fn configured_prompt_accept_and_cancel_execute_once_without_old_key_leakage() {
    let mut app = configured_prompt(
        "ui: {command-prompt: {accept: [F9], cancel: [F10]}}",
        PromptKind::Command,
        "buffer-new",
    );
    let before = app.buffers.len();
    for key in ["Enter", "Shift-Enter", "Esc", "Ctrl-c", "Shift-F9"] {
        prompt_key(&mut app, key);
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.command, "buffer-new");
        assert_eq!(app.buffers.len(), before);
    }
    prompt_key(&mut app, "F9");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.buffers.len(), before + 1);
    app.open_prompt_with_value(PromptKind::Command, "buffer-new".into());
    prompt_key(&mut app, "F10");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.buffers.len(), before + 1);
}

#[test]
fn prompt_editing_actions_preserve_reset_and_modifier_differences_when_remapped() {
    for kind in [
        PromptKind::Command,
        PromptKind::Rename,
        PromptKind::FinderPath,
        PromptKind::ExternalProgram,
    ] {
        let mut app = configured_prompt("{}", kind, "one two");
        for (key, cursor, selection, text) in [
            ("Ctrl-h", 6, 3, "one tw"),
            ("Backspace", 6, 0, "one tw"),
            ("Ctrl-Alt-b", 6, 3, "one two"),
            ("Alt-b", 4, 3, "one two"),
            ("Ctrl-Up", 7, 3, "one two"),
            ("Ctrl-n", 7, 3, "one two"),
            ("Space", 8, 0, "one two "),
        ] {
            app.command = "one two".into();
            app.command_cursor = 7;
            app.command_selection = 3;
            prompt_key(&mut app, key);
            assert_eq!(
                (
                    app.command_cursor,
                    app.command_selection,
                    app.command.as_str()
                ),
                (cursor, selection, text),
                "{kind:?} {key}"
            );
        }
    }
    let mut app = configured_prompt(
        "ui: {text-prompt: {delete-backward: [F6], backspace: [F7], word-left: [F8]}}",
        PromptKind::Rename,
        "one two",
    );
    app.command_selection = 3;
    prompt_key(&mut app, "Ctrl-h");
    prompt_key(&mut app, "Backspace");
    assert_eq!(app.command, "one two");
    prompt_key(&mut app, "F6");
    assert_eq!((app.command.as_str(), app.command_selection), ("one tw", 3));
    prompt_key(&mut app, "F7");
    assert_eq!((app.command.as_str(), app.command_selection), ("one t", 0));
    prompt_key(&mut app, "F8");
    assert_eq!(app.command_cursor, 4);
}

#[test]
fn prompt_accept_authority_distinguishes_default_modified_keys_and_explicit_assignments() {
    let input = |key: &str| InputEvent::Key(KeyStroke::parse(key).unwrap());
    let mut app = configured_prompt("{}", PromptKind::Command, "buffer-new");
    assert!(app.fresh_physical_approval_key(&input("Enter")));
    assert!(!app.fresh_physical_approval_key(&input("Shift-Enter")));
    app = configured_prompt(
        "ui: {command-prompt: {accept: [Ctrl-F9]}}",
        PromptKind::Command,
        "buffer-new",
    );
    assert!(app.fresh_physical_approval_key(&input("Ctrl-F9")));
    assert!(!app.fresh_physical_approval_key(&input("Enter")));
    assert!(!app.fresh_physical_approval_key(&input("F9")));
    assert!(!app.fresh_physical_approval_key(&InputEvent::Text("\n".into())));
    app.path_popup = Some(PathPopup {
        path: "example.txt".into(),
    });
    assert!(!app.fresh_physical_approval_key(&input("Ctrl-F9")));
}

#[test]
fn prompt_cursor_and_deletion_actions_preserve_text_and_choice_boundaries() {
    for kind in [PromptKind::Command, PromptKind::Rename] {
        let mut app = configured_prompt("{}", kind, "one two");
        for (key, cursor, selection, text) in [
            ("Ctrl-d", 4, 3, "one wo"),
            ("Ctrl-w", 0, 3, "two"),
            ("Ctrl-u", 0, 3, "two"),
            ("Ctrl-k", 4, 3, "one "),
            ("Ctrl-a", 0, 3, "one two"),
            ("Ctrl-e", 7, 3, "one two"),
            ("Ctrl-f", 5, 3, "one two"),
            ("Alt-f", 7, 3, "one two"),
            ("Left", 3, 3, "one two"),
            ("Right", 5, 3, "one two"),
        ] {
            app.command = "one two".into();
            app.command_cursor = 4;
            app.command_selection = 3;
            prompt_key(&mut app, key);
            assert_eq!(
                (
                    app.command_cursor,
                    app.command_selection,
                    app.command.as_str()
                ),
                (cursor, selection, text),
                "{kind:?} {key}"
            );
        }
        app.command = "one two".into();
        app.command_cursor = 4;
        app.command_selection = 3;
        prompt_key(&mut app, "Delete");
        assert_eq!(
            app.command,
            if kind == PromptKind::Command {
                "one two"
            } else {
                "one wo"
            }
        );
        assert_eq!(
            app.command_selection,
            if kind == PromptKind::Command { 3 } else { 0 }
        );
        prompt_key(&mut app, "Home");
        assert_eq!(
            (app.command_cursor, app.command_selection),
            if kind == PromptKind::Command {
                (4, 0)
            } else {
                (0, 0)
            }
        );
    }
}

#[test]
fn configured_prompt_hints_and_program_navigation_use_the_active_owner() {
    let mut app = configured_prompt(
        "ui: {command-prompt: {accept: [F9], cancel: [F10]}, program-prompt: {next: [F6], previous: [F7], open-actions: [F8], accept: [F9]}}",
        PromptKind::Command,
        "buffer-new",
    );
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::CommandPalette)
        .unwrap();
    assert_eq!(
        snapshot
            .actions
            .iter()
            .map(|a| a.key_hint.as_str())
            .collect::<Vec<_>>(),
        ["F9", "F10"]
    );
    // No cache directory: this fixture only exercises in-memory choices.
    app.programs = ProgramCache::load(None);
    app.programs.remember("example-viewer").unwrap();
    app.open_prompt_with_value(PromptKind::ExternalProgram, "example-viewer".into());
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::ProgramHints)
        .unwrap();
    assert!(snapshot.title.contains("F9 open · F8 actions"));
    prompt_key(&mut app, "F6");
    assert_eq!(
        app.command_selection, 0,
        "one choice clamps instead of wrapping"
    );
    prompt_key(&mut app, "Tab");
    assert!(app.program_action_menu.is_none());
    prompt_key(&mut app, "F8");
    assert!(app.program_action_menu.is_some());
    assert_eq!(app.command, "example-viewer");
}

#[test]
fn covered_prompt_does_not_grant_configured_accept_authority() {
    let input = InputEvent::Key(KeyStroke::parse("F9").unwrap());
    let mut app = configured_prompt(
        "ui: {command-prompt: {accept: [F9]}}",
        PromptKind::Command,
        "buffer-new",
    );
    app.jump = crate::jump_labels::JumpLabels::new([0]);
    assert!(!app.fresh_physical_approval_key(&input));
    app.jump = None;
    app.macro_replay = Some(MacroReplay {
        root_register: 'a',
        frames: vec![],
        commands: vec![],
        remaining_work: 0,
        processed_work: 0,
        abort_reason: None,
        last_action_error: false,
    });
    assert!(!app.fresh_physical_approval_key(&input));
    app.macro_replay = None;
    assert!(app.fresh_physical_approval_key(&input));
}
