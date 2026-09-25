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
