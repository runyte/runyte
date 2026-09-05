// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn saving_a_read_only_buffer_is_a_warning() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_about();

    app.save(None, false).unwrap();

    assert_eq!(app.status, "this buffer is read-only");
    assert_eq!(app.unread_notification_counts().warnings, 1);
    assert_eq!(app.unread_notification_counts().errors, 0);
    assert_eq!(app.unread_notification_counts().infos, 0);
}

#[test]
fn saving_over_a_file_changed_on_disk_is_a_warning() {
    let directory = temporary("save-conflict-warning");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notes.txt");
    fs::write(&path, "baseline\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let buffer = app.active().buffer;
    app.apply_to_buffer(buffer, &Transaction::insert(0, "local "));
    fs::write(&path, "external\n").unwrap();

    app.save(None, false).unwrap();

    assert!(app.status.contains("changed on disk"), "{}", app.status);
    assert_eq!(app.unread_notification_counts().warnings, 1);
    assert_eq!(app.unread_notification_counts().errors, 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), "external\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dirty_file_reload_is_confirmed_and_installs_only_the_reviewed_revision() {
    let directory = temporary("dirty-file-reload-confirmation");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notes.txt");
    fs::write(&path, "disk one\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let buffer = app.active().buffer;
    app.apply_to_buffer(buffer, &Transaction::insert(0, "local "));
    let history = app.buffers[buffer].history_len();
    fs::write(&path, "disk two\n").unwrap();

    app.reload_file().unwrap();
    assert!(app.file_reload_confirmation.is_some());
    assert_eq!(app.buffers[buffer].to_string(), "local disk one\n");
    assert_eq!(app.buffers[buffer].history_len(), history);
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Reload file");
    assert!(overlay.message.unwrap().contains("Space b d"));

    fs::write(&path, "disk three\n").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.file_reload_confirmation.is_some());
    assert_eq!(app.buffers[buffer].to_string(), "local disk one\n");
    assert!(app.status.contains("review reload again"), "{}", app.status);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.file_reload_confirmation.is_none());
    assert_eq!(app.buffers[buffer].to_string(), "disk three\n");
    assert!(!app.buffers[buffer].dirty);
    assert_eq!(app.buffers[buffer].history_len(), 0);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_reload_confirmation_cannot_land_on_a_buffer_closed_after_review() {
    let directory = temporary("closed-file-reload-confirmation");
    fs::create_dir_all(&directory).unwrap();
    let reviewed = directory.join("reviewed.txt");
    let other = directory.join("other.txt");
    fs::write(&reviewed, "reviewed on disk\n").unwrap();
    fs::write(&other, "other on disk\n").unwrap();
    let mut app = App::new(Config::default(), Some(reviewed.clone())).unwrap();
    let reviewed_buffer = app.active().buffer;
    app.apply_to_buffer(reviewed_buffer, &Transaction::insert(0, "unsaved "));
    fs::write(&reviewed, "new disk revision\n").unwrap();
    app.reload_file().unwrap();
    assert!(app.file_reload_confirmation.is_some());

    // A host-side close can overtake an attached client's confirmation and
    // explicitly discard the dirty buffer before the delayed Enter arrives.
    app.open_file(other.clone()).unwrap();
    let other_buffer = app.active().buffer;
    app.host_close_buffer(reviewed_buffer, true).unwrap();
    assert!(app.closed_buffers.contains(&reviewed_buffer));

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.file_reload_confirmation.is_none());
    assert!(app.status_error);
    assert!(
        app.status.contains("file buffer was closed"),
        "{}",
        app.status
    );
    assert_eq!(app.active().buffer, other_buffer);
    assert_eq!(app.buffers[other_buffer].to_string(), "other on disk\n");
    assert_eq!(
        fs::read_to_string(&reviewed).unwrap(),
        "new disk revision\n"
    );
    assert_eq!(fs::read_to_string(&other).unwrap(), "other on disk\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_state_is_shared_by_panes_and_transported_snapshots() {
    let directory = temporary("stale-shared-snapshot");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notes.txt");
    fs::write(&path, "baseline\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    let buffer = app.active().buffer;
    app.apply_to_buffer(buffer, &Transaction::insert(0, "local "));
    fs::write(&path, "external\n").unwrap();
    let event = app.buffers[buffer].observe_now(buffer).unwrap();
    app.apply_file_observation(event.clone());
    app.apply_file_observation(event);

    let view = app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
    let snapshot = app.snapshot(&view);
    assert!(snapshot.panes.iter().all(|pane| {
        pane.title.external_file_status == crate::buffer::ExternalFileStatus::Changed
    }));
    assert_eq!(
        snapshot.status.external_file_status,
        crate::buffer::ExternalFileStatus::Changed
    );
    assert_eq!(app.unread_notification_counts().warnings, 1);
    let wire: crate::protocol::EditorSnapshot = snapshot.into();
    assert!(wire.panes.iter().all(|pane| {
        pane.title.external_file_status == crate::protocol::ExternalFileStatus::Changed
    }));
    fs::remove_dir_all(directory).unwrap();
}

/// Applies one freshly read observation of whatever the buffer projects.
fn observe(app: &mut App, buffer: usize) {
    let event = app.buffers[buffer].observe_now(buffer).unwrap();
    app.apply_file_observation(event);
}

fn explorer_status(app: &App, buffer: usize) -> crate::buffer::ExternalFileStatus {
    app.buffers[buffer].external_file_status()
}

#[test]
fn external_children_make_an_explorer_listing_stale_until_it_is_refreshed() {
    let directory = temporary("explorer-stale-listing");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("a.txt"), "a\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_explorer(Some(directory.clone())).unwrap();
    let buffer = app.active().buffer;
    let listed = app.buffers[buffer].to_string();

    observe(&mut app, buffer);
    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Synchronized
    );

    fs::write(directory.join("b.txt"), "b\n").unwrap();
    observe(&mut app, buffer);
    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Changed
    );
    // Detection reports; it does not re-read the rows the person is looking at.
    assert_eq!(app.buffers[buffer].to_string(), listed);

    app.refresh_directory().unwrap();
    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Synchronized
    );
    observe(&mut app, buffer);
    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Synchronized
    );

    fs::remove_file(directory.join("b.txt")).unwrap();
    observe(&mut app, buffer);
    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Changed
    );

    app.refresh_directory().unwrap();
    fs::rename(directory.join("a.txt"), directory.join("c.txt")).unwrap();
    observe(&mut app, buffer);
    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Changed
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_hidden_entry_outside_the_listing_does_not_make_it_stale() {
    let directory = temporary("explorer-stale-hidden");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("a.txt"), "a\n").unwrap();
    let mut config = Config::default();
    config.editor.show_hidden_files = false;
    let mut app = App::new(config, None).unwrap();
    app.open_explorer(Some(directory.clone())).unwrap();
    let buffer = app.active().buffer;

    fs::write(directory.join(".hidden"), "x\n").unwrap();
    observe(&mut app, buffer);

    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Synchronized
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_stale_explorer_marks_every_pane_and_keeps_its_unsaved_edits() {
    let directory = temporary("explorer-stale-panes");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("a.txt"), "a\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_explorer(Some(directory.clone())).unwrap();
    let buffer = app.active().buffer;
    app.split(Axis::Horizontal, None).unwrap();
    assert!(
        app.panes.values().all(|pane| pane.buffer == buffer),
        "both panes must show the explorer"
    );
    app.apply_to_buffer(buffer, &Transaction::insert(0, "renamed-"));
    let edited = app.buffers[buffer].to_string();

    fs::write(directory.join("b.txt"), "b\n").unwrap();
    observe(&mut app, buffer);

    let view = app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
    let snapshot = app.snapshot(&view);
    assert!(snapshot.panes.len() >= 2);
    assert!(snapshot.panes.iter().all(|pane| {
        pane.title.external_file_status == crate::buffer::ExternalFileStatus::Changed
    }));
    assert_eq!(app.buffers[buffer].to_string(), edited);
    assert!(app.buffers[buffer].dirty);

    // The edit is still undoable: nothing replaced the projection.
    app.undo();
    assert_ne!(app.buffers[buffer].to_string(), edited);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn an_observation_of_the_listing_being_replaced_cannot_re_mark_a_refreshed_explorer() {
    let directory = temporary("explorer-stale-generation");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("a.txt"), "a\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_explorer(Some(directory.clone())).unwrap();
    let buffer = app.active().buffer;

    fs::write(directory.join("b.txt"), "b\n").unwrap();
    let in_flight = app.buffers[buffer].observe_now(buffer).unwrap();
    app.refresh_directory().unwrap();
    app.apply_file_observation(in_flight);

    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Synchronized
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_removed_directory_is_reported_without_discarding_the_explorer() {
    let directory = temporary("explorer-stale-removed");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("a.txt"), "a\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_explorer(Some(directory.clone())).unwrap();
    let buffer = app.active().buffer;
    let listed = app.buffers[buffer].to_string();

    fs::remove_dir_all(&directory).unwrap();
    observe(&mut app, buffer);

    assert_eq!(
        explorer_status(&app, buffer),
        crate::buffer::ExternalFileStatus::Deleted
    );
    assert_eq!(app.buffers[buffer].to_string(), listed);
}

#[cfg(unix)]
#[test]
fn opening_a_symlink_alias_reuses_the_live_file_buffer() {
    use std::os::unix::fs::symlink;

    let directory = temporary("open-symlink-alias");
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("target.txt");
    let alias = directory.join("alias.txt");
    fs::write(&target, "original\n").unwrap();
    symlink("target.txt", &alias).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();

    app.open_file(alias).unwrap();
    let buffer = app.active().buffer;
    app.apply_to_buffer(buffer, &Transaction::insert(0, "unsaved "));
    app.open_file(target).unwrap();

    assert_eq!(app.active().buffer, buffer);
    assert_eq!(app.active_buffer().to_string(), "unsaved original\n");
    assert_eq!(
        app.buffers
            .iter()
            .enumerate()
            .filter(|(index, buffer)| {
                !app.closed_buffers.contains(index) && buffer.path.is_some()
            })
            .count(),
        1,
        "one resolved file identity must own one live buffer"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn force_save_as_refuses_a_path_owned_by_another_live_buffer() {
    let directory = temporary("save-as-live-buffer-collision");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first.txt");
    let second = directory.join("second.txt");
    fs::write(&first, "first on disk\n").unwrap();
    fs::write(&second, "second on disk\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(first.clone()).unwrap();
    let first_buffer = app.active().buffer;
    app.open_file(second.clone()).unwrap();
    let second_buffer = app.active().buffer;
    app.apply_to_buffer(second_buffer, &Transaction::insert(0, "edited "));

    app.save(Some(first.clone()), true).unwrap();

    assert!(app.status_error);
    assert!(app.status.contains("already open"), "{}", app.status);
    assert_eq!(fs::read_to_string(&first).unwrap(), "first on disk\n");
    assert_eq!(
        app.buffers[first_buffer].path.as_deref(),
        Some(first.as_path())
    );
    assert_eq!(
        app.buffers[second_buffer].path.as_deref(),
        Some(second.as_path())
    );
    assert!(app.buffers[second_buffer].dirty);
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn saving_refuses_a_second_buffer_that_converged_on_the_same_file() {
    use std::os::unix::fs::symlink;

    let directory = temporary("save-converged-live-buffer-collision");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first.txt");
    let second = directory.join("second.txt");
    fs::write(&first, "first on disk\n").unwrap();
    fs::write(&second, "second on disk\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(first.clone()).unwrap();
    let first_buffer = app.active().buffer;
    app.open_file(second.clone()).unwrap();
    let second_buffer = app.active().buffer;
    app.open_file(first.clone()).unwrap();
    app.apply_to_buffer(first_buffer, &Transaction::insert(0, "edited "));

    fs::remove_file(&second).unwrap();
    symlink("first.txt", &second).unwrap();
    app.save(None, true).unwrap();

    assert!(app.status_error);
    assert!(app.status.contains("already open"), "{}", app.status);
    assert_eq!(fs::read_to_string(&first).unwrap(), "first on disk\n");
    assert_eq!(app.active().buffer, first_buffer);
    assert_eq!(
        app.buffers[second_buffer].path.as_deref(),
        Some(second.as_path())
    );
    assert!(app.buffers[first_buffer].dirty);
    fs::remove_dir_all(directory).unwrap();
}

// -- Jumplist ----------------------------------------------------------

#[test]
fn opening_a_file_records_a_jump_that_leads_back_and_forward_again() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "scratch line\nsecond\n");
    set_cursor(&mut app, 1, 3);

    let path = temporary_directory().join(format!("runyte-jump-{}.txt", std::process::id()));
    app.open_file(path).unwrap();
    let opened = app.active().buffer;
    assert_ne!(opened, 0, "a second buffer should be open");

    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(app.active().buffer, 0);
    assert_eq!(cursor(&app), Position::new(1, 3), "the selection returns");

    key(&mut app, KeyCode::Char('i'), Modifiers::CONTROL);
    assert_eq!(app.active().buffer, opened);

    // Tab now owns contextual actions. It cannot move through history,
    // even when this buffer has no action to offer.
    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(app.active().buffer, 0);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.active().buffer, 0);
    assert_eq!(app.status, "No actions available for this selection");
    key(&mut app, KeyCode::Char('i'), Modifiers::CONTROL);
    assert_eq!(app.active().buffer, opened);
}

#[test]
fn tab_does_not_bypass_a_command_waiting_for_a_character() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abc\n");

    press(&mut app, 'r');
    assert_eq!(
        app.awaiting_character_command(),
        Some(EditorCommand::ReplaceChar)
    );
    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    assert_eq!(app.active_buffer().to_string(), "abc\n");
    assert!(app.context_action_menu.is_none());
    assert!(app.awaiting_character_command().is_none());
    assert_eq!(app.status, "expected a character");

    press(&mut app, 'z');
    assert_eq!(app.active_buffer().to_string(), "abc\n");
}

/// Closing returns to the buffer this pane displayed most recently.
#[test]
fn closing_a_buffer_wraps_to_the_first_live_buffer() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-close-{}.txt", std::process::id()));
    std::fs::write(&path, "the file being read\n").unwrap();

    app.open_file(path.clone()).unwrap();
    assert_ne!(
        app.active().buffer,
        0,
        "the file is not the startup scratch"
    );

    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    assert!(app.active_buffer().is_help());

    app.execute_command("bc").unwrap();
    assert_eq!(app.active_buffer().path.as_deref(), Some(path.as_path()));

    std::fs::remove_file(path).unwrap();
}

/// Reading a long document records a jump per section, so `Ctrl-o` has to
/// step through all of them to leave it. `Alt-o` skips straight out.
#[test]
fn alt_o_and_alt_i_step_between_buffers_skipping_positions_within_one() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "scratch line\nsecond\n");
    set_cursor(&mut app, 1, 3);

    let path = temporary_directory().join(format!("runyte-jump-file-{}.txt", std::process::id()));
    std::fs::write(&path, "alpha\nbeta\ngamma\ndelta\nepsilon\n").unwrap();
    app.open_file(path.clone()).unwrap();
    let opened = app.active().buffer;

    // Three long-range motions inside the opened file, each a jump.
    for row in [4, 0, 3] {
        set_cursor(&mut app, row, 0);
        press(&mut app, 'g');
        press(&mut app, 'g');
        set_cursor(&mut app, row, 0);
        app.push_jump();
    }
    assert!(
        app.active().jumps.len() > 3,
        "the file should hold several recorded positions"
    );

    // Ctrl-o walks the positions inside the file rather than leaving it.
    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(
        app.active().buffer,
        opened,
        "Ctrl-o steps within the file rather than out of it"
    );

    // One Alt-o leaves the file outright, however many remain behind it.
    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);
    assert_eq!(app.active().buffer, 0, "one step left the file");
    assert_eq!(cursor(&app), Position::new(1, 3), "the selection returns");

    // Alt-i comes back to a recorded position in the file, not its top.
    key(&mut app, KeyCode::Char('i'), Modifiers::ALT);
    assert_eq!(app.active().buffer, opened);

    std::fs::remove_file(path).unwrap();
}

#[test]
fn the_ends_of_the_jumplist_report_rather_than_move() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one\ntwo\nthree\n");
    set_cursor(&mut app, 2, 0);

    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(app.status, "no earlier position");
    assert_eq!(cursor(&app), Position::new(2, 0), "nothing moved");

    key(&mut app, KeyCode::Char('i'), Modifiers::CONTROL);
    assert_eq!(app.status, "no later position");
    assert_eq!(cursor(&app), Position::new(2, 0));
}

#[test]
fn local_motion_records_nothing_but_a_file_boundary_does() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one\ntwo\nthree\nfour\n");

    for _ in 0..3 {
        press(&mut app, 'j');
    }
    press(&mut app, 'l');
    assert!(
        app.active().jumps.is_empty(),
        "walking through a file is not a jump"
    );

    assert_eq!(cursor(&app), Position::new(3, 1));

    // `gg` can cross an arbitrary distance, so it is.
    press(&mut app, 'g');
    press(&mut app, 'g');
    assert_eq!(cursor(&app), Position::new(0, 0));
    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(cursor(&app), Position::new(3, 1));
}

#[test]
fn starting_a_search_records_a_jump_but_repeating_it_does_not() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbeta\nbeta\nbeta\n");

    press(&mut app, '/');
    type_text(&mut app, "beta");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(cursor(&app).row, 1);
    assert_eq!(app.active().jumps.len(), 1);

    press(&mut app, 'n');
    press(&mut app, 'n');
    assert_eq!(cursor(&app).row, 3);
    assert_eq!(
        app.active().jumps.len(),
        1,
        "stepping through matches must not bury where the search began"
    );

    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(cursor(&app), Position::new(0, 0));
}

#[test]
fn an_edit_moves_a_remembered_position_with_the_text() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one\ntwo\nthree\n");
    set_cursor(&mut app, 2, 0);
    press(&mut app, 'g');
    press(&mut app, 'g');

    // Two lines inserted above the remembered position.
    press(&mut app, 'i');
    type_text(&mut app, "zero\nhalf\n");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(
        cursor(&app),
        Position::new(4, 0),
        "the jump followed the line it pointed at"
    );
}

#[test]
fn a_language_server_jump_is_reachable_by_going_back() {
    let (mut app, path, _queue) = rust_app("fn one() {}\nfn two() {}\nfn three() {}\n");
    ready(&mut app, Encoding::Utf8);
    set_cursor(&mut app, 0, 3);

    app.lsp_requests.insert(
        1,
        tracked(
            &app,
            PendingRequest::Goto {
                label: "definition",
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 1,
        response: Response::Locations(vec![crate::lsp::Location {
            path,
            range: LspRange::new(LspPosition::new(2, 3), LspPosition::new(2, 8)),
            encoding: Encoding::Utf8,
        }]),
    });
    assert_eq!(cursor(&app).row, 2);

    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(
        cursor(&app),
        Position::new(0, 3),
        "following a definition must be reversible"
    );
}

#[test]
fn explorer_entry_points_open_editable_directories_and_keep_picker_distinct() {
    let directory = temporary("explorer-entry-points");
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();
    app.working_directory = directory.clone();
    press(&mut app, ' ');
    press(&mut app, 'e');
    assert!(app.active_buffer().is_directory());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.as_path())
    );
    assert!(app.picker.is_none());

    press(&mut app, ' ');
    press(&mut app, 'f');
    assert!(app.picker.is_some());

    app.picker = None;
    app.execute_command("explorer child").unwrap();
    assert!(app.active_buffer().is_directory());
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn goto_file_infers_the_complete_absolute_path_at_every_kind_of_character() {
    let directory = temporary("goto-file-absolute");
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("file.txt");
    let source = directory.join("source.txt");
    fs::write(&target, "target\n").unwrap();
    let path = target.display().to_string();
    fs::write(&source, format!("{path}\n")).unwrap();

    for column in [
        path.chars().position(|character| character == 'i').unwrap(),
        path.chars().position(|character| character == '/').unwrap(),
        path.chars().count() - 1,
    ] {
        let mut app = App::new(Config::default(), Some(source.clone())).unwrap();
        set_cursor(&mut app, 0, column);
        press(&mut app, 'g');
        press(&mut app, 'f');
        assert_eq!(app.active_buffer().path.as_deref(), Some(target.as_path()));
    }

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn goto_file_uses_an_exact_selection_and_asks_between_relative_matches() {
    let root = temporary("goto-file-relative");
    let active_directory = root.join("active");
    fs::create_dir_all(&active_directory).unwrap();
    let project_match = root.join("file.txt");
    let active_match = active_directory.join("file.txt");
    let source = active_directory.join("source.txt");
    fs::write(&project_match, "project\n").unwrap();
    fs::write(&active_match, "active\n").unwrap();
    fs::write(&source, "prefix/file.txt/suffix\n").unwrap();

    let mut app = App::new(Config::default(), Some(source)).unwrap();
    app.project_root = root.clone();
    let start = "prefix/".chars().count();
    let end = start + "file.txt".chars().count();
    app.active_mut()
        .replace_selection(Selection::single(Range::new(start, end - 1)));
    press(&mut app, 'g');
    press(&mut app, 'f');

    let picker = app.list.as_ref().expect("two matches open a picker");
    assert_eq!(picker.title, "Go to file");
    assert_eq!(picker.items.len(), 2);
    assert_eq!(picker.items[0].label, active_match.display().to_string());
    assert_eq!(picker.items[1].label, project_match.display().to_string());

    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.list.is_none());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(project_match.as_path())
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn goto_file_opens_a_selected_directory_as_an_explorer() {
    let root = temporary("goto-file-directory");
    let destination = root.join("destination");
    fs::create_dir_all(&destination).unwrap();
    let source = root.join("source.txt");
    let path = destination.display().to_string();
    fs::write(&source, format!("{path}/file.txt\n")).unwrap();

    let mut app = App::new(Config::default(), Some(source)).unwrap();
    app.active_mut()
        .replace_selection(Selection::single(Range::new(0, path.chars().count() - 1)));
    press(&mut app, 'g');
    press(&mut app, 'f');

    assert!(app.active_buffer().is_directory());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(destination.as_path())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_picker_lists_directories_and_enter_opens_the_explorer() {
    let directory = temporary("picker-directory-entry");
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(directory.join("file.txt"), "hello").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    let mut picker = FilePicker::new(
        1,
        directory.clone(),
        crate::file_picker::ScanScope::ignoring(&directory),
    );
    picker.add_paths(vec![
        ScanEntry::directory(child.clone()),
        ScanEntry::file(directory.join("file.txt")),
    ]);
    picker.finish(0, false);
    app.picker = Some(picker);

    press(&mut app, '/');
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(
        picker.matches.len(),
        1,
        "trailing slash narrows to directories"
    );
    assert!(
        picker.selected_entry().is_some_and(|entry| entry.is_dir),
        "the only remaining match is the directory entry"
    );
    assert_eq!(picker.selected_entry().unwrap().label(), "child/");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.picker.is_none());
    assert!(app.active_buffer().is_directory());
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_and_explorer_entry_points_use_their_documented_roots() {
    let root = temporary("cd-from-file");
    let file_directory = root.join("files");
    let destination = root.join("destination");
    fs::create_dir_all(&file_directory).unwrap();
    fs::create_dir_all(&destination).unwrap();
    let file = file_directory.join("note.txt");
    fs::write(&file, "hello").unwrap();

    let mut app = App::new(Config::default(), Some(file.clone())).unwrap();
    app.project_root = root.clone();
    type_command(&mut app, &format!("cd {}", destination.display()));

    assert_eq!(app.working_directory, destination);
    assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));

    press(&mut app, ' ');
    press(&mut app, 'f');
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.root.as_path()),
        Some(root.as_path()),
        "the finder searches the stable project root"
    );

    app.picker = None;
    type_command(&mut app, "file-picker-directory");
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.root.as_path()),
        Some(file_directory.as_path()),
        "the directory picker searches beside the active file"
    );

    app.picker = None;
    press(&mut app, ' ');
    press(&mut app, 'e');
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(file_directory.as_path()),
        "lowercase e opens beside the file in the active pane"
    );

    app.open_file(file).unwrap();
    press(&mut app, ' ');
    press(&mut app, 'E');
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(destination.as_path()),
        "uppercase E opens the directory selected by :cd"
    );

    type_command(&mut app, "file-picker-directory");
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.root.as_path()),
        Some(destination.as_path()),
        "an explorer searches its current root"
    );

    app.picker = None;
    press(&mut app, ' ');
    press(&mut app, 'b');
    press(&mut app, 'n');
    assert!(matches!(app.active_buffer().kind, BufferKind::Scratch));
    type_command(&mut app, "file-picker-directory");
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.root.as_path()),
        Some(destination.as_path()),
        "a pathless buffer searches the working directory"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn space_e_keeps_a_navigated_explorer_in_its_current_directory() {
    let root = temporary("explorer-current-directory");
    let current = root.join("current");
    let working = root.join("working");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&working).unwrap();

    let mut app = App::new(Config::default(), Some(current.clone())).unwrap();
    app.working_directory = working;

    press(&mut app, ' ');
    press(&mut app, 'e');
    assert_eq!(app.active_buffer().path.as_deref(), Some(current.as_path()));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cd_retargets_an_active_explorer_and_resolves_relative_paths() {
    let root = temporary("cd-from-explorer");
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();

    let mut app = App::new(Config::default(), Some(first.clone())).unwrap();
    app.working_directory = root.clone();
    let explorer = app.active().buffer;
    type_command(&mut app, "cd second");

    assert_eq!(app.working_directory, second);
    assert_eq!(app.active().buffer, explorer, "the pane keeps one explorer");
    assert_eq!(app.active_buffer().path.as_deref(), Some(second.as_path()));
    assert!(app.status.contains("working directory:"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explorer_yank_and_paste_copies_into_a_retargeted_pane_on_write() {
    let parent = temporary("explorer-copy-paste");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("note.txt"), "copied content").unwrap();
    let mut app = App::new(Config::default(), Some(source.clone())).unwrap();

    press(&mut app, 'x');
    press(&mut app, 'y');
    app.open_file(destination.clone()).unwrap();
    press(&mut app, 'p');

    assert_eq!(app.active_buffer().to_string(), "note.txt\n");
    assert!(matches!(
        app.active_buffer().directory_plan().unwrap().operations(),
        [FsOperation::Copy { from, to, .. }]
            if from == Path::new("../source/note.txt") && to == Path::new("note.txt")
    ));
    press(&mut app, 'u');
    assert!(app.active_buffer().directory_plan().unwrap().is_empty());
    press(&mut app, 'U');
    assert!(matches!(
        app.active_buffer().directory_plan().unwrap().operations(),
        [FsOperation::Copy { .. }]
    ));
    app.execute_command("write").unwrap();
    assert!(app.fs_confirmation.is_some());
    assert!(!destination.join("note.txt").exists());

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        fs::read_to_string(destination.join("note.txt")).unwrap(),
        "copied content"
    );
    assert!(source.join("note.txt").exists());
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn filesystem_plan_review_supports_single_step_page_and_boundary_navigation() {
    let root = temporary("filesystem-confirmation-navigation");
    fs::create_dir_all(&root).unwrap();
    let snapshot = crate::fs_plan::DirectorySnapshot::read(&root).unwrap();
    let desired = (0..25)
        .map(|index| {
            crate::fs_plan::DesiredEntry::create(
                format!("entry-{index:02}"),
                crate::fs_plan::EntryKind::File,
            )
        })
        .collect();
    let plan = crate::fs_plan::FsPlan::build(root.clone(), snapshot, desired).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.fs_confirmation = Some(FsConfirmation {
        buffer: app.active().buffer,
        plan,
        selected: 0,
    });

    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    assert_eq!(app.fs_confirmation.as_ref().unwrap().selected, 2);
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    key(&mut app, KeyCode::Char('p'), Modifiers::CONTROL);
    assert_eq!(app.fs_confirmation.as_ref().unwrap().selected, 0);

    key(&mut app, KeyCode::PageDown, Modifiers::NONE);
    key(&mut app, KeyCode::Char('d'), Modifiers::CONTROL);
    assert_eq!(app.fs_confirmation.as_ref().unwrap().selected, 20);
    key(&mut app, KeyCode::PageUp, Modifiers::NONE);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(app.fs_confirmation.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::End, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(app.fs_confirmation.as_ref().unwrap().selected, 24);
    key(&mut app, KeyCode::Home, Modifiers::NONE);
    key(&mut app, KeyCode::Char('x'), Modifiers::NONE);
    assert_eq!(app.fs_confirmation.as_ref().unwrap().selected, 0);

    key(&mut app, KeyCode::Char('c'), Modifiers::CONTROL);
    assert!(app.fs_confirmation.is_none());
    assert_eq!(app.status, "filesystem plan cancelled");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explorer_yank_of_a_bare_caret_takes_the_whole_entry() {
    let parent = temporary("explorer-caret-yank");
    let source = parent.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("note.txt"), "copied content").unwrap();
    let mut app = App::new(Config::default(), Some(source)).unwrap();

    press(&mut app, 'y');

    // In a file buffer a bare `y` takes one character; here the caret
    // names the entry on its row, so the text has to agree with the file
    // identity the register carries beside it.
    assert_eq!(app.registers[&'"'].text, "note.txt\n");
    assert!(app.registers[&'"'].linewise);
    assert!(app.registers[&'"'].directory.is_some());

    // `Y` reaches the register by the other path, so it has to arrive at
    // the same entry rather than at the row's text alone.
    app.registers.clear();
    press(&mut app, 'Y');
    assert_eq!(app.registers[&'"'].text, "note.txt\n");
    assert!(app.registers[&'"'].linewise);
    assert!(app.registers[&'"'].directory.is_some());

    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn explorer_paste_between_existing_rows_keeps_their_identities() {
    let parent = temporary("explorer-middle-paste");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("note.txt"), "copied content").unwrap();
    fs::write(destination.join("a.txt"), "a").unwrap();
    fs::write(destination.join("z.txt"), "z").unwrap();
    let mut app = App::new(Config::default(), Some(source)).unwrap();

    press(&mut app, 'x');
    press(&mut app, 'y');
    app.open_file(destination.clone()).unwrap();
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'p');

    assert!(matches!(
        app.active_buffer().directory_plan().unwrap().operations(),
        [FsOperation::Copy { to, .. }] if to == Path::new("note.txt")
    ));
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(fs::read_to_string(destination.join("a.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(destination.join("z.txt")).unwrap(), "z");
    assert_eq!(
        fs::read_to_string(destination.join("note.txt")).unwrap(),
        "copied content"
    );
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn explorer_line_selection_copies_multiple_entries_with_helix_keys() {
    let parent = temporary("explorer-multi-copy");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("a.txt"), "a").unwrap();
    fs::write(source.join("b.txt"), "b").unwrap();
    let mut app = App::new(Config::default(), Some(source)).unwrap();

    press(&mut app, 'x');
    press(&mut app, 'x');
    press(&mut app, 'y');
    app.open_file(destination.clone()).unwrap();
    press(&mut app, 'p');

    let plan = app.active_buffer().directory_plan().unwrap();
    assert_eq!(
        plan.operations()
            .iter()
            .filter(|operation| matches!(operation, FsOperation::Copy { .. }))
            .count(),
        2
    );
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(fs::read_to_string(destination.join("a.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(destination.join("b.txt")).unwrap(), "b");
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn explorer_copy_can_be_renamed_in_the_same_directory_before_write() {
    let directory = temporary("explorer-same-directory-copy");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("note.txt"), "copied content").unwrap();
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();

    press(&mut app, 'x');
    press(&mut app, 'y');
    press(&mut app, 'p');
    set_cursor(&mut app, 1, 0);
    press(&mut app, 'x');
    press(&mut app, 'c');
    type_text(&mut app, "note-copy.txt");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.execute_command("write").unwrap();

    assert!(matches!(
        app.fs_confirmation
            .as_ref()
            .unwrap()
            .plan
            .operations(),
        [FsOperation::Copy { from, to, .. }]
            if from == Path::new("note.txt") && to == Path::new("note-copy.txt")
    ));
    assert!(!directory.join("note-copy.txt").exists());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        fs::read_to_string(directory.join("note-copy.txt")).unwrap(),
        "copied content"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn deleting_the_original_of_a_same_directory_copy_becomes_a_rename() {
    let directory = temporary("explorer-copy-then-delete-source");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("note.txt"), "moved content").unwrap();
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();

    press(&mut app, 'x');
    press(&mut app, 'y');
    press(&mut app, 'p');
    set_cursor(&mut app, 1, 0);
    press(&mut app, 'x');
    press(&mut app, 'c');
    type_text(&mut app, "renamed.txt");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'x');
    press(&mut app, 'd');
    app.execute_command("write").unwrap();

    assert!(matches!(
        app.fs_confirmation
            .as_ref()
            .unwrap()
            .plan
            .operations(),
        [FsOperation::Rename { from, to, .. }]
            if from == Path::new("note.txt") && to == Path::new("renamed.txt")
    ));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!directory.join("note.txt").exists());
    assert_eq!(
        fs::read_to_string(directory.join("renamed.txt")).unwrap(),
        "moved content"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explorer_delete_and_paste_moves_across_panes_on_write() {
    let parent = temporary("explorer-cut-paste");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("note.txt"), "moved content").unwrap();
    let mut app = App::new(Config::default(), Some(source.clone())).unwrap();
    let source_buffer = app.active().buffer;

    press(&mut app, 'x');
    press(&mut app, 'd');
    assert!(app.buffers[source_buffer].dirty);
    app.split(Axis::Horizontal, Some(destination.clone()))
        .unwrap();
    press(&mut app, 'p');
    app.execute_command("write").unwrap();
    assert!(matches!(
        app.fs_confirmation
            .as_ref()
            .unwrap()
            .plan
            .operations(),
        [FsOperation::Move { from, to, .. }]
            if from == Path::new("../source/note.txt") && to == Path::new("note.txt")
    ));
    assert!(source.join("note.txt").exists());
    assert!(!destination.join("note.txt").exists());

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(!source.join("note.txt").exists());
    assert_eq!(
        fs::read_to_string(destination.join("note.txt")).unwrap(),
        "moved content"
    );
    assert!(!app.buffers[source_buffer].dirty);
    assert_eq!(app.buffers[source_buffer].to_string(), "");
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn writing_a_cut_source_cannot_delete_a_move_pasted_in_another_pane() {
    let parent = temporary("explorer-source-write-before-move");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("note.txt"), "moved content").unwrap();
    let mut app = App::new(Config::default(), Some(source.clone())).unwrap();
    let source_buffer = app.active().buffer;

    press(&mut app, 'x');
    press(&mut app, 'd');
    app.split(Axis::Horizontal, Some(destination.clone()))
        .unwrap();
    let destination_pane = app.active_pane;
    press(&mut app, 'p');

    app.active_pane = 0;
    app.execute_command("write").unwrap();

    assert!(app.fs_confirmation.is_none());
    assert!(app.status_error, "{}", app.status);
    assert!(app.status.contains("write the destination first"));
    assert!(source.join("note.txt").is_file());
    assert!(app.buffers[source_buffer].dirty);

    app.active_pane = destination_pane;
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(!source.join("note.txt").exists());
    assert_eq!(
        fs::read_to_string(destination.join("note.txt")).unwrap(),
        "moved content"
    );
    assert!(!app.buffers[source_buffer].dirty);
    assert!(!app.status_error, "{}", app.status);
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn a_textually_clean_explorer_still_owns_its_pending_move() {
    let parent = temporary("explorer-clean-pending-move");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("note.txt"), "moved content").unwrap();
    fs::write(destination.join("note.txt"), "replaced content").unwrap();
    let mut app = App::new(Config::default(), Some(source.clone())).unwrap();
    let source_buffer = app.active().buffer;
    let transfer = app.buffers[source_buffer]
        .directory_transfer_at(0)
        .unwrap()
        .unwrap();

    press(&mut app, 'x');
    press(&mut app, 'd');
    app.split(Axis::Horizontal, Some(destination)).unwrap();
    let destination_buffer = app.active().buffer;
    app.buffers[destination_buffer]
        .assign_directory_transfers(0, &[transfer], TransferMode::Move)
        .unwrap();
    assert!(
        !app.buffers[destination_buffer].dirty,
        "changing a row's hidden origin does not change its saved text"
    );

    app.active_pane = 0;
    app.execute_command("write").unwrap();

    assert!(app.fs_confirmation.is_none());
    assert!(app.status.contains("write the destination first"));
    assert!(source.join("note.txt").is_file());
    assert!(app.buffers[source_buffer].dirty);
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn deleting_a_directory_cannot_invalidate_a_nested_pending_move() {
    let parent = temporary("explorer-delete-parent-of-pending-move");
    let source = parent.join("source");
    let child = source.join("child");
    let destination = parent.join("destination");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(child.join("note.txt"), "moved content").unwrap();
    let mut app = App::new(Config::default(), Some(parent.clone())).unwrap();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'x');
    press(&mut app, 'd');
    let child_buffer = Buffer::open_directory(&child, ListingView::default()).unwrap();
    let transfer = child_buffer.directory_transfer_at(0).unwrap().unwrap();
    let mut destination_buffer =
        Buffer::open_directory(&destination, ListingView::default()).unwrap();
    assert!(destination_buffer.apply(&Transaction::insert(0, "note.txt\n")));
    destination_buffer
        .assign_directory_transfers(0, &[transfer], TransferMode::Move)
        .unwrap();
    app.buffers.push(child_buffer);
    app.syntax.push(None);
    app.buffers.push(destination_buffer);
    app.syntax.push(None);

    app.execute_command("write").unwrap();

    assert!(app.fs_confirmation.is_none());
    assert!(app.status.contains("write the destination first"));
    assert!(child.join("note.txt").is_file());
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn renaming_a_directory_cannot_invalidate_a_nested_pending_move() {
    let parent = temporary("explorer-rename-parent-of-pending-move");
    let source = parent.join("source");
    let child = source.join("child");
    let destination = parent.join("destination");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(child.join("note.txt"), "moved content").unwrap();
    let mut app = App::new(Config::default(), Some(parent.clone())).unwrap();

    let source_start = app
        .active_buffer()
        .to_string()
        .find("source/")
        .expect("source row is listed");
    assert!(app.buffers[0].apply(&Transaction::change(
        source_start,
        source_start + "source".len(),
        "renamed",
    )));
    let child_buffer = Buffer::open_directory(&child, ListingView::default()).unwrap();
    let transfer = child_buffer.directory_transfer_at(0).unwrap().unwrap();
    let mut destination_buffer =
        Buffer::open_directory(&destination, ListingView::default()).unwrap();
    assert!(destination_buffer.apply(&Transaction::insert(0, "note.txt\n")));
    destination_buffer
        .assign_directory_transfers(0, &[transfer], TransferMode::Move)
        .unwrap();
    app.buffers.push(child_buffer);
    app.syntax.push(None);
    app.buffers.push(destination_buffer);
    app.syntax.push(None);

    app.execute_command("write").unwrap();

    assert!(app.fs_confirmation.is_none());
    assert!(app.status.contains("write the destination first"));
    assert!(child.join("note.txt").is_file());
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn writing_a_cut_that_was_not_pasted_still_deletes_it() {
    let directory = temporary("explorer-unpasted-cut-delete");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("note.txt"), "deleted content").unwrap();
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();

    press(&mut app, 'x');
    press(&mut app, 'd');
    app.execute_command("write").unwrap();

    assert!(matches!(
        app.fs_confirmation
            .as_ref()
            .expect("an unpasted cut remains an ordinary deletion")
            .plan
            .operations(),
        [FsOperation::Delete { path, .. }] if path == Path::new("note.txt")
    ));
    key(&mut app, KeyCode::Char('P'), Modifiers::NONE);
    assert!(!directory.join("note.txt").exists());
    assert!(!app.active_buffer().dirty);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_mixed_source_plan_waits_for_its_pasted_move_then_keeps_other_edits() {
    let parent = temporary("explorer-mixed-source-write-before-move");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("a.txt"), "kept content").unwrap();
    fs::write(source.join("b.txt"), "moved content").unwrap();
    let mut app = App::new(Config::default(), Some(source.clone())).unwrap();
    let source_buffer = app.active().buffer;

    assert!(app.buffers[source_buffer].apply(&Transaction::new(vec![
        Change::new(0, "a.txt".len(), "renamed.txt"),
        Change::new(
            "a.txt\nb.txt\n".len(),
            "a.txt\nb.txt\n".len(),
            "created.txt\n"
        ),
    ])));
    set_cursor(&mut app, 1, 0);
    press(&mut app, 'x');
    press(&mut app, 'd');
    app.split(Axis::Horizontal, Some(destination.clone()))
        .unwrap();
    let destination_pane = app.active_pane;
    press(&mut app, 'p');

    app.active_pane = 0;
    app.execute_command("write").unwrap();
    assert!(app.fs_confirmation.is_none());
    assert!(app.status.contains("write the destination first"));
    assert!(source.join("b.txt").is_file());

    app.active_pane = destination_pane;
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    let remaining = app.buffers[source_buffer].directory_plan().unwrap();
    assert_eq!(
        remaining.operations().len(),
        2,
        "{:?}",
        remaining.operations()
    );
    assert!(remaining.operations().iter().any(|operation| matches!(
        operation,
        FsOperation::Rename { from, to, .. }
            if from == Path::new("a.txt") && to == Path::new("renamed.txt")
    )));
    assert!(remaining.operations().iter().any(|operation| matches!(
        operation,
        FsOperation::Create { path, .. } if path == Path::new("created.txt")
    )));

    app.active_pane = 0;
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(!source.join("a.txt").exists());
    assert_eq!(
        fs::read_to_string(source.join("renamed.txt")).unwrap(),
        "kept content"
    );
    assert!(source.join("created.txt").is_file());
    assert_eq!(
        fs::read_to_string(destination.join("b.txt")).unwrap(),
        "moved content"
    );
    assert!(!app.buffers[source_buffer].dirty);
    assert!(!app.status_error, "{}", app.status);
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn a_cross_pane_move_rebases_other_source_edits_before_their_write() {
    let parent = temporary("explorer-mixed-cross-pane");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("moved-a.txt"), "first moved content").unwrap();
    fs::write(source.join("moved-b.txt"), "second moved content").unwrap();
    let mut app = App::new(Config::default(), Some(source.clone())).unwrap();
    let source_buffer = app.active().buffer;

    let end = app.buffers[source_buffer].len_chars();
    assert!(app.buffers[source_buffer].apply(&Transaction::insert(end, "created.txt\n")));
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'x');
    press(&mut app, 'x');
    press(&mut app, 'd');
    let source_plan = app.buffers[source_buffer].directory_plan().unwrap();
    assert_eq!(source_plan.operations().len(), 3);
    assert!(matches!(
        source_plan.operations().first(),
        Some(FsOperation::Create { path, .. }) if path == Path::new("created.txt")
    ));
    assert_eq!(
        source_plan
            .operations()
            .iter()
            .filter(|operation| matches!(operation, FsOperation::Delete { .. }))
            .count(),
        2
    );

    app.split(Axis::Horizontal, Some(destination.clone()))
        .unwrap();
    press(&mut app, 'p');
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(!source.join("moved-a.txt").exists());
    assert!(!source.join("moved-b.txt").exists());
    assert_eq!(
        fs::read_to_string(destination.join("moved-a.txt")).unwrap(),
        "first moved content"
    );
    assert_eq!(
        fs::read_to_string(destination.join("moved-b.txt")).unwrap(),
        "second moved content"
    );
    assert!(app.buffers[source_buffer].dirty);
    let remaining_plan = app.buffers[source_buffer].directory_plan().unwrap();
    assert!(
        matches!(
            remaining_plan.operations(),
            [FsOperation::Create { path, .. }] if path == Path::new("created.txt")
        ),
        "{:?} · {}",
        remaining_plan.operations(),
        app.status
    );

    app.active_pane = 0;
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(source.join("created.txt").is_file());
    assert!(!app.buffers[source_buffer].dirty);
    assert!(!app.status_error, "{}", app.status);
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn a_pending_explorer_cut_can_follow_same_pane_navigation() {
    let parent = temporary("explorer-same-pane-cut");
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("note.txt"), "moved content").unwrap();
    let mut app = App::new(Config::default(), Some(source.clone())).unwrap();

    press(&mut app, 'x');
    press(&mut app, 'd');
    app.open_file(destination.clone()).unwrap();

    assert!(app.directory_reload_confirmation.is_none());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(destination.as_path())
    );
    press(&mut app, 'p');
    app.execute_command("write").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!source.join("note.txt").exists());
    assert_eq!(
        fs::read_to_string(destination.join("note.txt")).unwrap(),
        "moved content"
    );
    fs::remove_dir_all(parent).unwrap();
}

/// Entering a directory reads it, and the read can fail. The pane must not
/// have taken the explorer over by then.
#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_listed_adopts_no_explorer() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary("explorer-failed-entry");
    let locked = directory.join("locked");
    let note = directory.join("note.txt");
    fs::create_dir_all(&locked).unwrap();
    fs::write(&note, "hello").unwrap();

    // One explorer showing `locked`, then step off it onto a file so
    // nothing is displaying or reserving it. Re-entering it now has to
    // re-read the listing, which is the read that will fail.
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    app.open_file(locked.clone()).unwrap();
    let explorer = app.active().buffer;
    app.open_file(note).unwrap();
    app.active_mut().directory_buffer = None;
    let showing = app.active().buffer;
    let buffers = app.buffers.len();
    assert_ne!(showing, explorer);

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        // A user permissions do not restrain, so there is nothing to fail.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let _ = fs::remove_dir_all(&directory);
        return;
    }
    let refused = app.open_file(locked.clone());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let _ = fs::remove_dir_all(&directory);

    assert!(refused.is_err(), "an unreadable directory was entered");
    assert_eq!(
        app.active().directory_buffer,
        None,
        "a directory that could not be listed was still taken over"
    );
    assert_eq!(
        app.active().buffer,
        showing,
        "a directory that could not be listed still moved the pane"
    );
    assert_eq!(
        app.buffers.len(),
        buffers,
        "a directory that could not be listed still added a buffer"
    );
}

/// Wait activation becomes a Normal-mode boundary only after its target has
/// opened successfully. A refused request must leave the existing editor
/// interaction exactly where it was.
#[cfg(unix)]
#[test]
fn a_failed_directory_wait_preserves_the_existing_mode_and_selection() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary("wait-failed-directory-entry");
    let locked = directory.join("locked");
    let note = directory.join("note.txt");
    fs::create_dir_all(&locked).unwrap();
    fs::write(&note, "hello").unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    app.open_file(locked.clone()).unwrap();
    app.open_file(note).unwrap();
    app.active_mut().directory_buffer = None;
    press(&mut app, 'i');
    let buffer = app.active().buffer;
    let selection = app.active().selection.clone();

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let _ = fs::remove_dir_all(&directory);
        return;
    }
    let refused = app.host_open_wait_files(vec![locked.clone()], true, &HashSet::new());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(refused.is_err(), "an unreadable directory wait was opened");
    assert_eq!(app.active().buffer, buffer);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.active().selection, selection);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_successful_wait_records_the_source_as_normal_in_jump_history() {
    let directory = temporary("wait-normal-jump");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.txt");
    let target = directory.join("target.txt");
    fs::write(&source, "alpha\n").unwrap();
    fs::write(&target, "beta\n").unwrap();

    let mut expected = App::new(Config::default(), Some(source.clone())).unwrap();
    press(&mut expected, 'v');
    press(&mut expected, 'l');
    expected.push_jump();
    key(&mut expected, KeyCode::Escape, Modifiers::NONE);
    expected.open_file(target.clone()).unwrap();
    expected.jump(true);

    let mut wait = App::new(Config::default(), Some(source)).unwrap();
    press(&mut wait, 'v');
    press(&mut wait, 'l');
    // The raw Select position already being newest exercises jumplist
    // deduplication: wait activation must append the distinct Normal form.
    wait.push_jump();
    wait.host_open_wait_files(vec![target], true, &HashSet::new())
        .unwrap();
    wait.jump(true);

    assert_eq!(wait.mode, Mode::Normal);
    assert_eq!(wait.active().selection, expected.active().selection);
    assert_eq!(
        wait.active().selection_semantics(),
        expected.active().selection_semantics()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_directory_wait_remembers_the_source_view_as_normal() {
    let directory = temporary("wait-normal-directory-view");
    let first = directory.join("first");
    let second = directory.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("alpha.txt"), "alpha\n").unwrap();
    fs::write(second.join("beta.txt"), "beta\n").unwrap();

    let mut expected = App::new(Config::default(), Some(first.clone())).unwrap();
    press(&mut expected, 'v');
    press(&mut expected, 'l');
    key(&mut expected, KeyCode::Escape, Modifiers::NONE);
    expected.open_file(second.clone()).unwrap();
    expected.open_file(first.clone()).unwrap();

    let mut wait = App::new(Config::default(), Some(first.clone())).unwrap();
    press(&mut wait, 'v');
    press(&mut wait, 'l');
    wait.host_open_wait_files(vec![second], true, &HashSet::new())
        .unwrap();
    wait.open_file(first).unwrap();

    assert_eq!(wait.mode, Mode::Normal);
    assert_eq!(wait.active().selection, expected.active().selection);
    fs::remove_dir_all(directory).unwrap();
}

/// A pane browses with one explorer however deep it walks, and the row it
/// left each directory on comes back with the directory.
#[test]
fn directory_navigation_retargets_one_buffer_and_preserves_each_view() {
    let directory = temporary("explorer-navigation");
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("note.txt"), "hello").unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let explorer = app.active().buffer;
    let buffers = app.buffers.len();
    let child_row = app
        .active_buffer()
        .lines()
        .position(|line| line == "child/")
        .unwrap();
    set_cursor(&mut app, child_row, 0);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));
    assert_eq!(app.active().buffer, explorer, "the same buffer, retargeted");
    assert_eq!(app.buffers.len(), buffers, "and no second directory buffer");

    let file_row = app
        .active_buffer()
        .lines()
        .position(|line| line == "note.txt")
        .unwrap();
    set_cursor(&mut app, file_row, 0);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(child.join("note.txt").as_path())
    );

    // The recently used clean explorer remains available. Reopening its
    // directory reuses the same pane-owned buffer and remembered view.
    assert!(!app.closed_buffers.contains(&explorer));
    app.open_file(child.clone()).unwrap();
    let reopened = app.active().buffer;
    assert_eq!(reopened, explorer);
    assert_eq!(app.cursor_position().row, file_row);
    press(&mut app, '-');
    assert_eq!(app.active().buffer, reopened);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.as_path())
    );
    assert_eq!(app.cursor_position().row, child_row);
    assert_eq!(
        app.buffers
            .iter()
            .enumerate()
            .filter(|(index, buffer)| {
                !app.closed_buffers.contains(index) && buffer.is_directory()
            })
            .count(),
        1,
        "walking the tree must leave one explorer behind, not one per directory"
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn active_directory_explorer_selects_the_file_it_was_opened_from() {
    let directory = temporary("explorer-focus-active-file");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("alpha.txt"), "alpha").unwrap();
    let file = directory.join("target.txt");
    fs::write(&file, "target").unwrap();
    let file = file.canonicalize().unwrap();
    let mut app = App::new(Config::default(), Some(file.clone())).unwrap();
    let file_buffer = app.active().buffer;

    press(&mut app, ' ');
    press(&mut app, 'e');

    assert!(app.active_buffer().is_directory());
    assert_eq!(
        app.selected_directory_entry().unwrap().as_deref(),
        Some(file.as_path())
    );
    assert_ne!(
        app.cursor_position().row,
        0,
        "the target is not the first row"
    );

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, file_buffer);
    assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn confirmed_active_directory_explorer_still_selects_the_file() {
    let first = temporary("explorer-focus-active-file-first");
    let second = temporary("explorer-focus-active-file-second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("alpha.txt"), "alpha").unwrap();
    let file = second.join("target.txt");
    fs::write(&file, "target").unwrap();
    let file = file.canonicalize().unwrap();
    let mut app = App::new(Config::default(), Some(first.clone())).unwrap();
    let explorer = app.active().buffer;
    app.buffers[explorer].apply(&Transaction::insert(0, "draft.txt\n"));
    app.open_file(file.clone()).unwrap();
    let file_buffer = app.active().buffer;

    press(&mut app, ' ');
    press(&mut app, 'e');
    assert_eq!(
        app.directory_reload_confirmation
            .as_ref()
            .and_then(|confirmation| confirmation.focus_entry.as_deref()),
        Some(file.as_path())
    );

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.selected_directory_entry().unwrap().as_deref(),
        Some(file.as_path())
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, file_buffer);

    fs::remove_dir_all(first).unwrap();
    fs::remove_dir_all(second).unwrap();
}

#[test]
fn parent_navigation_selects_the_child_without_a_saved_parent_view() {
    let parent = temporary("explorer-parent-focus-new-view");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(parent.join("sibling")).unwrap();
    let mut app = App::new(Config::default(), Some(child.clone())).unwrap();

    press(&mut app, '-');

    assert_eq!(app.active_buffer().path.as_deref(), Some(parent.as_path()));
    assert_eq!(
        app.selected_directory_entry().unwrap().as_deref(),
        Some(child.as_path())
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));

    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn parent_navigation_selects_the_child_over_an_older_saved_row() {
    let parent = temporary("explorer-parent-focus-saved-view");
    let child = parent.join("child");
    let sibling = parent.join("sibling");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&sibling).unwrap();
    let mut app = App::new(Config::default(), Some(parent.clone())).unwrap();
    let sibling_row = app
        .active_buffer()
        .lines()
        .position(|line| line == "sibling/")
        .unwrap();
    set_cursor(&mut app, sibling_row, 0);

    app.open_file(child.clone()).unwrap();
    press(&mut app, '-');

    assert_eq!(
        app.selected_directory_entry().unwrap().as_deref(),
        Some(child.as_path()),
        "the child just left takes precedence over the older saved row"
    );

    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn confirmed_parent_navigation_still_selects_the_child() {
    let parent = temporary("explorer-parent-focus-confirmed");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();
    let mut app = App::new(Config::default(), Some(child.clone())).unwrap();
    let explorer = app.active().buffer;
    app.buffers[explorer].apply(&Transaction::insert(0, "draft.txt\n"));

    press(&mut app, '-');
    assert_eq!(
        app.directory_reload_confirmation,
        Some(DirectoryReloadConfirmation {
            buffer: explorer,
            destination: Some(parent.clone()),
            focus_entry: Some(child.clone()),
        })
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.active_buffer().path.as_deref(), Some(parent.as_path()));
    assert_eq!(
        app.selected_directory_entry().unwrap().as_deref(),
        Some(child.as_path())
    );

    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn parent_navigation_keeps_the_fallback_view_when_the_child_is_filtered() {
    let parent = temporary("explorer-parent-focus-filtered");
    let child = parent.join(".child");
    let visible = parent.join("visible");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&visible).unwrap();
    let mut app = App::new(Config::default(), Some(child)).unwrap();

    press(&mut app, '-');

    assert_eq!(app.active_buffer().path.as_deref(), Some(parent.as_path()));
    assert_eq!(
        app.selected_directory_entry().unwrap().as_deref(),
        Some(visible.as_path()),
        "an absent hidden child must leave the ordinary initial view intact"
    );

    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn explorer_navigation_returns_to_normal_mode_after_selected_entries() {
    let directory = temporary("explorer-selection-mode");
    let child = directory.join("child");
    let file = directory.join("note.txt");
    fs::create_dir_all(&child).unwrap();
    fs::write(&file, "hello").unwrap();
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();

    // Searching in an explorer creates a Select-mode match. Entering its
    // directory must not carry that selection into the new listing.
    press(&mut app, '/');
    for character in "child".chars() {
        press(&mut app, character);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Select);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));
    assert_eq!(app.mode, Mode::Normal);

    // The parent-directory command has the same boundary when invoked
    // from a manually extended selection.
    press(&mut app, 'v');
    assert_eq!(app.mode, Mode::Select);
    press(&mut app, '-');
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.as_path())
    );
    assert_eq!(app.mode, Mode::Normal);

    let file_row = app
        .active_buffer()
        .lines()
        .position(|line| line == "note.txt")
        .unwrap();
    set_cursor(&mut app, file_row, 0);
    press(&mut app, 'v');
    assert_eq!(app.mode, Mode::Select);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));
    assert_eq!(app.mode, Mode::Normal);

    fs::remove_dir_all(directory).unwrap();
}

/// Retargeting throws the listing away, so unsaved edits to it are a
/// decision the person has to make rather than one navigation makes for
/// them.
#[test]
fn navigating_away_from_a_dirty_explorer_asks_before_discarding() {
    let directory = temporary("explorer-dirty-navigation");
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let explorer = app.active().buffer;
    let child_row = app
        .active_buffer()
        .lines()
        .position(|line| line == "child/")
        .unwrap();
    let end = app.active_buffer().len_chars();
    app.buffers[explorer].apply(&Transaction::insert(end, "draft.txt\n"));
    set_cursor(&mut app, child_row, 0);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.directory_reload_confirmation,
        Some(DirectoryReloadConfirmation {
            buffer: explorer,
            destination: Some(child.clone()),
            focus_entry: None,
        })
    );
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Discard directory edits");
    assert_eq!(overlay.actions[0].label, "discard and open");
    assert!(
        overlay
            .message
            .as_deref()
            .is_some_and(|message| message.contains(&child.display().to_string()))
    );
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.as_path()),
        "the navigation waits"
    );

    // Esc keeps the edits and stays put.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.directory_reload_confirmation, None);
    assert!(app.active_buffer().to_string().contains("draft.txt"));
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.as_path())
    );

    // Enter discards them and completes the navigation.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.directory_reload_confirmation, None);
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));
    assert!(!app.active_buffer().dirty);
    assert!(!app.active_buffer().to_string().contains("draft.txt"));

    fs::remove_dir_all(directory).unwrap();
}

/// Two panes are two views, so each keeps its own explorer and neither can
/// move the other's directory out from under it.
#[test]
fn each_pane_browses_with_an_explorer_of_its_own() {
    let directory = temporary("explorer-per-pane");
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let first_pane = app.active_pane;
    let first_explorer = app.active().buffer;

    app.split(Axis::Horizontal, None).unwrap();
    assert_ne!(app.active_pane, first_pane);
    assert_eq!(
        app.active().buffer,
        first_explorer,
        "the split starts shared"
    );

    let child_row = app
        .active_buffer()
        .lines()
        .position(|line| line == "child/")
        .unwrap();
    set_cursor(&mut app, child_row, 0);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));
    assert_ne!(
        app.active().buffer,
        first_explorer,
        "the second pane must not retarget the first pane's explorer"
    );
    assert_eq!(
        app.buffers[first_explorer].path.as_deref(),
        Some(directory.as_path()),
        "the first pane still shows the directory it was on"
    );
    assert_eq!(app.buffers.iter().filter(|b| b.is_directory()).count(), 2);

    fs::remove_dir_all(directory).unwrap();
}

/// Reaching a directory through the buffer picker is still navigation, so
/// the pane has to stop reserving whichever explorer it left behind.
#[test]
fn switching_to_a_directory_buffer_hands_over_the_panes_explorer() {
    let directory = temporary("explorer-switch");
    let child = directory.join("child");
    let sibling = directory.join("sibling");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&sibling).unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let root_explorer = app.active().buffer;
    app.split(Axis::Horizontal, None).unwrap();
    app.open_file(child.clone()).unwrap();
    let second_explorer = app.active().buffer;
    assert_ne!(second_explorer, root_explorer);

    // Back to the first pane's explorer through the buffer list. The
    // second pane owns it, so this pane takes no explorer at all.
    app.switch_buffer(root_explorer);
    assert_eq!(app.active().buffer, root_explorer);
    assert_eq!(app.active().directory_buffer, None);

    // Navigating on must therefore leave the first pane where it was, and
    // reclaim the explorer this pane walked away from rather than open a
    // third.
    app.open_file(sibling.clone()).unwrap();
    assert_eq!(app.active().buffer, second_explorer);
    assert_eq!(app.active_buffer().path.as_deref(), Some(sibling.as_path()));
    assert_eq!(
        app.buffers[root_explorer].path.as_deref(),
        Some(directory.as_path()),
        "the pane that owns the root explorer keeps it"
    );
    assert_eq!(app.buffers.iter().filter(|b| b.is_directory()).count(), 2);

    fs::remove_dir_all(directory).unwrap();
}

/// The other half: a directory buffer nobody is browsing with is adopted
/// on the way in, so the pane carries on retargeting it in place.
#[test]
fn switching_to_an_unclaimed_directory_buffer_adopts_it() {
    let directory = temporary("explorer-adopt-on-switch");
    let child = directory.join("child");
    let sibling = directory.join("sibling");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&sibling).unwrap();
    let note = directory.join("note.txt");
    fs::write(&note, "hello").unwrap();

    // One pane, one explorer, then step off it onto a file so nothing is
    // displaying or reserving it.
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    app.open_file(child.clone()).unwrap();
    let explorer = app.active().buffer;
    app.open_file(note).unwrap();
    app.active_mut().directory_buffer = None;
    assert_ne!(app.active().buffer, explorer);

    app.switch_buffer(explorer);
    assert_eq!(app.active().buffer, explorer);
    assert_eq!(app.active().directory_buffer, Some(explorer));

    let directories = app.buffers.iter().filter(|b| b.is_directory()).count();
    app.open_file(sibling.clone()).unwrap();
    assert_eq!(app.active().buffer, explorer, "retargeted, not replaced");
    assert_eq!(app.active_buffer().path.as_deref(), Some(sibling.as_path()));
    assert_eq!(
        app.buffers.iter().filter(|b| b.is_directory()).count(),
        directories
    );

    fs::remove_dir_all(directory).unwrap();
}

/// Retargeting swaps the whole listing outside the transaction system, so
/// nothing remaps these offsets; a jump back into them means nothing.
#[test]
fn retargeting_an_explorer_retires_jumps_into_it() {
    let directory = temporary("explorer-jumps");
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let explorer = app.active().buffer;
    let child_row = app
        .active_buffer()
        .lines()
        .position(|line| line == "child/")
        .unwrap();
    set_cursor(&mut app, child_row, 0);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, explorer);
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));
    assert_eq!(
        app.active().jumps.len(),
        0,
        "no jump may point into a listing that was replaced"
    );

    // A jump out of the explorer into a file is still worth keeping.
    let file = child.join("note.txt");
    fs::write(&file, "hello").unwrap();
    app.open_file(file).unwrap();
    assert_eq!(app.active().jumps.len(), 1);

    fs::remove_dir_all(directory).unwrap();
}

/// Buffers are never removed, so a closed pane's explorer has to be
/// adopted rather than left to accumulate.
#[test]
fn a_closed_panes_explorer_is_adopted_instead_of_orphaned() {
    let directory = temporary("explorer-orphan");
    let child = directory.join("child");
    let sibling = directory.join("sibling");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&sibling).unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    app.open_file(child.clone()).unwrap();
    assert_eq!(app.buffers.iter().filter(|b| b.is_directory()).count(), 2);

    app.close_pane();
    app.open_file(sibling.clone()).unwrap();
    assert_eq!(
        app.buffers.iter().filter(|b| b.is_directory()).count(),
        2,
        "the orphan is reused, not added to"
    );
    assert_eq!(app.active_buffer().path.as_deref(), Some(sibling.as_path()));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn e_moves_by_word_inside_a_directory_buffer() {
    let directory = temporary("explorer-word-end");
    fs::create_dir_all(directory.join("child")).unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let row = app
        .active_buffer()
        .lines()
        .position(|line| line == "child/")
        .unwrap();
    set_cursor(&mut app, row, 0);

    press(&mut app, 'e');
    assert!(
        app.active_buffer().is_directory(),
        "e must not open the entry under the caret"
    );
    assert_eq!(app.cursor_position().row, row);
    assert!(app.cursor_position().col > 0, "e must have moved");

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn colon_help_opens_one_manual_and_space_question_mark_stays_contextual() {
    let directory = temporary("explorer-help");
    fs::create_dir_all(&directory).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    type_command(&mut app, "?");
    assert!(app.active_buffer().is_manual());
    assert!(
        app.active_buffer()
            .to_string()
            .starts_with("Help · RUNYTE\n")
    );
    let manual = app.active().buffer;

    // Help is a buffer, so an ordinary motion moves inside it rather than
    // dismissing it. Opening the explorer and contextual help beside it stays
    // well inside the retained set, so the manual is still there afterwards.
    press(&mut app, 'j');
    assert!(app.active_buffer().is_manual());

    // Contextual help continues to describe wherever Space ? is invoked.
    app.open_file(directory.clone()).unwrap();
    press(&mut app, ' ');
    press(&mut app, '?');
    assert!(!app.closed_buffers.contains(&manual));
    assert!(app.active_buffer().is_help());
    assert!(
        app.active_buffer()
            .to_string()
            .starts_with("Help · RUNYTE · EXPLORER")
    );

    press(&mut app, 'q');
    type_command(&mut app, "help regex");
    assert_eq!(
        app.active().buffer,
        manual,
        "a retained manual is reprojected in place"
    );
    assert_eq!(app.cursor_position().row, app.active().scroll_row);
    assert_eq!(
        app.active_buffer().line_string(app.cursor_position().row),
        "REGULAR EXPRESSIONS"
    );

    // Its Help-scoped q closes it like any other generated help page.
    press(&mut app, 'q');
    assert!(!app.active_buffer().is_manual());
    type_command(&mut app, "help missing-topic");
    assert!(app.status_error);
    assert!(app.status.starts_with("unknown help topic: missing-topic"));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn help_topic_tab_completion_uses_registered_canonical_slugs() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, ':');
    type_text(&mut app, "help reg");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "help regex");

    app.command = "? lang".to_owned();
    app.command_cursor = app.command.chars().count();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "? language-servers");
}

#[test]
fn about_command_opens_one_read_only_front_page() {
    let mut app = App::new(Config::default(), None).unwrap();

    type_command(&mut app, "about");

    assert_eq!(app.active_buffer().display_name(), "[about]");
    assert!(app.active_buffer().is_read_only());
    assert!(
        app.active_buffer()
            .to_string()
            .contains(&format!("Runyte {}", env!("CARGO_PKG_VERSION")))
    );
    assert!(app.active_buffer().to_string().contains("R U N Y T E"));
    let about = app.active().buffer;

    type_command(&mut app, "about");

    assert_eq!(app.active().buffer, about, "the front page is reused");
    press(&mut app, 'i');
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.status_error);
    assert_eq!(app.unread_notification_counts().infos, 1);
    assert_eq!(app.unread_notification_counts().errors, 0);
}

/// A text buffer has one help document, not one per mode. Select mode is
/// described inside it rather than answered with a separate topic, so the
/// palette dropping back to Normal before it runs cannot change the answer.
#[test]
fn help_for_a_text_buffer_is_the_same_document_in_every_mode() {
    let from_normal = {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, "hello world\n");
        press(&mut app, ' ');
        press(&mut app, '?');
        app.active_buffer().to_string()
    };

    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "hello world\n");
    press(&mut app, 'v');
    assert_eq!(app.mode, Mode::Select);
    press(&mut app, ' ');
    press(&mut app, '?');
    assert_eq!(app.active_buffer().to_string(), from_normal);

    // And through the key binding, which never leaves the mode at all.
    press(&mut app, 'q');
    press(&mut app, 'v');
    press(&mut app, ' ');
    press(&mut app, '?');
    assert_eq!(app.active_buffer().to_string(), from_normal);

    // The one document has to answer for the mode it no longer branches on.
    assert!(from_normal.starts_with("Help · RUNYTE · TEXT"));
    assert!(from_normal.contains("SELECT mode"), "{from_normal}");
    assert!(from_normal.contains("NORMAL mode"), "{from_normal}");
}

#[test]
fn directory_refresh_requires_confirmation_before_discarding_edits() {
    let directory = temporary("explorer-refresh");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("kept.txt"), "hello").unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let buffer = app.active().buffer;
    let end = app.active_buffer().len_chars();
    app.buffers[buffer].apply(&Transaction::insert(end, "draft.txt\n"));

    press(&mut app, ' ');
    press(&mut app, 'r');
    assert_eq!(
        app.directory_reload_confirmation,
        Some(DirectoryReloadConfirmation {
            buffer,
            destination: None,
            focus_entry: None,
        })
    );
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Discard directory edits");
    assert_eq!(overlay.actions[0].label, "discard and refresh");
    assert!(
        overlay
            .message
            .as_deref()
            .is_some_and(|message| message.contains("unsaved directory edits"))
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.active_buffer().dirty);
    assert!(app.active_buffer().to_string().contains("draft.txt"));

    press(&mut app, ' ');
    press(&mut app, 'r');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.active_buffer().dirty);
    assert!(!app.active_buffer().to_string().contains("draft.txt"));
    assert!(app.active_buffer().to_string().contains("kept.txt"));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn splitting_an_explorer_shows_the_same_listing_in_both_panes() {
    let root = temporary("explorer-split");
    let directory = root.join("work");
    fs::create_dir_all(directory.join("child")).unwrap();
    let file = directory.join("note.txt");
    fs::write(&file, "hello").unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let explorer = app.active().buffer;
    let row = app
        .active_buffer()
        .lines()
        .position(|line| line == "note.txt")
        .unwrap();
    set_cursor(&mut app, row, 0);
    press(&mut app, ' ');
    press(&mut app, 'w');
    press(&mut app, 'v');
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, 's');

    assert_eq!(app.panes.len(), 3);
    // The split keeps showing the directory it was made from, exactly as
    // splitting a file keeps showing the same text.
    assert_eq!(app.active().buffer, explorer);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.as_path())
    );
    assert_eq!(app.cursor_position().row, row);

    // An empty row is no longer a reason a split cannot be made.
    let last = app.active_buffer().last_row();
    set_cursor(&mut app, last, 0);
    press(&mut app, ' ');
    press(&mut app, 'w');
    press(&mut app, 'v');
    assert_eq!(app.panes.len(), 4);
    assert_eq!(app.active().buffer, explorer);

    // Each split still browses with an explorer of its own, so navigating
    // one leaves the other where it was: that is what keeps a copy across
    // two explorers possible.
    press(&mut app, '-');
    assert_ne!(app.active().buffer, explorer, "{}", app.status);
    assert_eq!(app.active_buffer().path.as_deref(), Some(root.as_path()));
    assert_eq!(
        app.buffers[explorer].path.as_deref(),
        Some(directory.as_path())
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vim_directory_scope_opens_entries_and_parents_before_global_interpretation() {
    let directory = temporary("vim-explorer-scope");
    fs::create_dir_all(directory.join("child")).unwrap();
    let file = directory.join("note.txt");
    fs::write(&file, "hello").unwrap();
    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;

    let mut opening = App::new(config.clone(), Some(directory.clone())).unwrap();
    let row = opening
        .active_buffer()
        .lines()
        .position(|line| line == "note.txt")
        .unwrap();
    set_cursor(&mut opening, row, 0);
    key(&mut opening, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        opening.active_buffer().path.as_deref(),
        Some(file.as_path())
    );

    let mut parenting = App::new(config, Some(directory.join("child"))).unwrap();
    key(&mut parenting, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(
        parenting.active_buffer().path.as_deref(),
        Some(directory.as_path())
    );
    key(&mut parenting, KeyCode::Char('G'), Modifiers::SHIFT);
    assert_eq!(
        parenting.cursor_position().row,
        parenting.active_buffer().last_row()
    );

    fs::remove_dir_all(directory).unwrap();
}

/// An editor-wait request arrives from a program running inside the terminal
/// the pane is showing, so covering that terminal with the requested document
/// is a detour rather than a decision to stop using the terminal. Finishing
/// with the document has to put the terminal back where it was.
#[cfg(unix)]
#[test]
fn quitting_a_wait_document_uncovers_the_terminal_it_replaced() {
    let directory = temporary("wait-over-terminal-quit");
    fs::create_dir_all(&directory).unwrap();
    let note = directory.join("note.txt");
    let merge = directory.join("MERGE_MSG");
    fs::write(&note, "alpha\n").unwrap();
    fs::write(&merge, "Merge branch 'dev'\n").unwrap();

    let mut app = App::new(Config::default(), Some(note)).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let pane = app.active_pane;

    app.host_open_wait_files(vec![merge], true, &HashSet::new())
        .unwrap();
    assert_eq!(app.terminal_of_pane(pane), None);
    assert_eq!(app.mode, Mode::Normal);

    app.execute_command("quit").unwrap();

    assert_eq!(app.panes.len(), 2, "the terminal's pane was closed");
    assert_eq!(app.active_pane, pane);
    assert_eq!(app.terminal_of_pane(pane), Some(terminal));
    assert_eq!(app.mode, Mode::Insert, "the terminal did not take input");
    fs::remove_dir_all(directory).unwrap();
}

/// The uncovering happens before every other reading of `:quit`, so the one
/// pane of a single-pane editor is not asked whether the whole editor should
/// stop.
#[cfg(unix)]
#[test]
fn quitting_a_wait_document_over_the_only_pane_uncovers_rather_than_exits() {
    let directory = temporary("wait-over-terminal-single-pane");
    fs::create_dir_all(&directory).unwrap();
    let merge = directory.join("MERGE_MSG");
    fs::write(&merge, "Merge branch 'dev'\n").unwrap();

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let pane = app.active_pane;

    app.host_open_wait_files(vec![merge], true, &HashSet::new())
        .unwrap();
    app.execute_command("quit").unwrap();

    assert!(!app.should_quit, "quitting the detour stopped the editor");
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.terminal_of_pane(pane), Some(terminal));
    fs::remove_dir_all(directory).unwrap();
}

/// `:close` and `:wbc` retire the requested buffer without touching the pane
/// layout, so the pane they leave behind is the terminal's, not an unrelated
/// fallback document.
#[cfg(unix)]
#[test]
fn closing_a_wait_document_uncovers_the_terminal_it_replaced() {
    let directory = temporary("wait-over-terminal-close");
    fs::create_dir_all(&directory).unwrap();
    let note = directory.join("note.txt");
    let merge = directory.join("MERGE_MSG");
    fs::write(&note, "alpha\n").unwrap();
    fs::write(&merge, "Merge branch 'dev'\n").unwrap();

    let mut app = App::new(Config::default(), Some(note)).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let pane = app.active_pane;

    app.host_open_wait_files(vec![merge], true, &HashSet::new())
        .unwrap();
    app.execute_command("close").unwrap();

    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.terminal_of_pane(pane), Some(terminal));
    fs::remove_dir_all(directory).unwrap();
}

/// Navigating the pane somewhere else is the person saying they are done with
/// the terminal, so the detour ends and `:quit` closes the pane as it always
/// does for a document.
#[cfg(unix)]
#[test]
fn navigating_away_from_a_wait_document_ends_the_terminal_detour() {
    let directory = temporary("wait-over-terminal-navigated");
    fs::create_dir_all(&directory).unwrap();
    let note = directory.join("note.txt");
    let other = directory.join("other.txt");
    let merge = directory.join("MERGE_MSG");
    fs::write(&note, "alpha\n").unwrap();
    fs::write(&other, "beta\n").unwrap();
    fs::write(&merge, "Merge branch 'dev'\n").unwrap();

    let mut app = App::new(Config::default(), Some(note)).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let pane = app.active_pane;

    app.host_open_wait_files(vec![merge], true, &HashSet::new())
        .unwrap();
    app.open_file(other).unwrap();
    app.execute_command("quit").unwrap();

    assert_eq!(app.panes.len(), 1, "the pane was kept for a left terminal");
    assert!(!app.panes.contains_key(&pane));
    assert!(
        app.terminals
            .get(terminal)
            .is_some_and(TerminalSession::live)
    );
    fs::remove_dir_all(directory).unwrap();
}

/// A terminal another pane has taken in the meantime is that pane's; the
/// detour cannot move it back out from under it.
#[cfg(unix)]
#[test]
fn a_terminal_taken_by_another_pane_is_not_uncovered_again() {
    let directory = temporary("wait-over-terminal-taken");
    fs::create_dir_all(&directory).unwrap();
    let note = directory.join("note.txt");
    let merge = directory.join("MERGE_MSG");
    fs::write(&note, "alpha\n").unwrap();
    fs::write(&merge, "Merge branch 'dev'\n").unwrap();

    let mut app = App::new(Config::default(), Some(note)).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let covered = app.active_pane;

    app.host_open_wait_files(vec![merge], true, &HashSet::new())
        .unwrap();
    let other = *app.panes.keys().find(|pane| **pane != covered).unwrap();
    app.activate_pane(other);
    app.show_terminal(terminal);
    app.activate_pane(covered);
    app.execute_command("quit").unwrap();

    assert_eq!(app.terminal_of_pane(other), Some(terminal));
    assert_eq!(app.panes.len(), 1, "the emptied pane was kept");
    fs::remove_dir_all(directory).unwrap();
}

/// An ordinary host open is the same kind of outside request as a wait: a
/// second client, or `runyte file` run elsewhere, did not ask this pane to
/// stop showing its terminal either.
#[cfg(unix)]
#[test]
fn a_host_open_over_a_terminal_is_also_a_detour() {
    let directory = temporary("host-open-over-terminal");
    fs::create_dir_all(&directory).unwrap();
    let note = directory.join("note.txt");
    let other = directory.join("other.txt");
    fs::write(&note, "alpha\n").unwrap();
    fs::write(&other, "beta\n").unwrap();

    let mut app = App::new(Config::default(), Some(note)).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let pane = app.active_pane;

    app.host_open_files(vec![other], true).unwrap();
    assert_eq!(app.terminal_of_pane(pane), None);

    app.execute_command("quit").unwrap();

    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.terminal_of_pane(pane), Some(terminal));
    fs::remove_dir_all(directory).unwrap();
}

/// A terminal's pane still holds the buffer it was showing before the
/// terminal opened, but that buffer is the pane's history rather than
/// something a split was asked for. Splitting a terminal asks for somewhere
/// to work, so the new pane opens the working-directory explorer.
#[cfg(unix)]
#[test]
fn splitting_a_terminal_opens_a_working_directory_explorer() {
    for axis in [Axis::Vertical, Axis::Horizontal] {
        let root = temporary("terminal-split-explorer");
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        let note = root.join("note.txt");
        fs::write(&note, "alpha\n").unwrap();

        let mut app = App::new(Config::default(), Some(note)).unwrap();
        app.working_directory = work.clone();
        let covered = app.active().buffer;
        app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
        let terminal = app.active_terminal().unwrap();
        let source = app.active_pane;
        // Leaving the child's input puts the terminal in review, which is
        // the state a `Space w v` split is made from.
        // A live terminal takes two presses to reach review: the first
        // leaves the child's input for live Normal, the second captures.
        key(&mut app, KeyCode::Char('\\'), Modifiers::CONTROL);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.terminals.get(terminal).unwrap().reviewing());
        key(&mut app, KeyCode::Char('\\'), Modifiers::CONTROL);
        assert!(app.terminals.get(terminal).unwrap().reviewing());

        app.split_window(axis).unwrap();

        assert_eq!(app.panes.len(), 2);
        assert_ne!(app.active_pane, source);
        assert_eq!(
            app.terminal_of_pane(source),
            Some(terminal),
            "the terminal left its original pane"
        );
        assert_eq!(app.active_terminal(), None);
        assert_ne!(
            app.active().buffer,
            covered,
            "the split reopened the buffer the terminal covered"
        );
        assert!(app.active_buffer().is_directory());
        assert_eq!(app.active_buffer().path.as_deref(), Some(work.as_path()));
        assert_eq!(app.mode, Mode::Normal);
        let child = app.terminals.get(terminal).unwrap();
        assert!(child.live());
        assert!(child.reviewing(), "the split ended the terminal's review");

        fs::remove_dir_all(root).unwrap();
    }
}

/// A split copies the pane it came from, and one terminal has one size, so
/// the copy must not also inherit the claim on it. Two panes racing to reveal
/// the same session is the failure the split's own terminal rule avoids.
#[cfg(unix)]
#[test]
fn a_split_does_not_inherit_the_claim_on_a_covered_terminal() {
    let directory = temporary("wait-over-terminal-split");
    fs::create_dir_all(&directory).unwrap();
    let note = directory.join("note.txt");
    let merge = directory.join("MERGE_MSG");
    fs::write(&note, "alpha\n").unwrap();
    fs::write(&merge, "Merge branch 'dev'\n").unwrap();

    let mut app = App::new(Config::default(), Some(note)).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let covered = app.active_pane;

    app.host_open_wait_files(vec![merge], true, &HashSet::new())
        .unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    let split = app.active_pane;
    assert_ne!(split, covered);

    app.execute_command("quit").unwrap();

    assert!(
        !app.panes.contains_key(&split),
        "the split kept itself open on a terminal it never showed"
    );
    assert_eq!(app.terminal_of_pane(covered), None);

    app.activate_pane(covered);
    app.execute_command("quit").unwrap();

    assert_eq!(app.terminal_of_pane(covered), Some(terminal));
    fs::remove_dir_all(directory).unwrap();
}

/// A claim is owed for the one document that covered the terminal. Once the
/// pane has fallen back to another buffer, retiring that buffer is not the
/// request being finished with, so it must not reveal the terminal.
#[cfg(unix)]
#[test]
fn a_spent_claim_does_not_reveal_the_terminal_under_a_later_buffer() {
    let directory = temporary("wait-over-terminal-superseded");
    fs::create_dir_all(&directory).unwrap();
    let note = directory.join("note.txt");
    let other = directory.join("other.txt");
    let merge = directory.join("MERGE_MSG");
    fs::write(&note, "alpha\n").unwrap();
    fs::write(&other, "beta\n").unwrap();
    fs::write(&merge, "Merge branch 'dev'\n").unwrap();

    let mut app = App::new(Config::default(), Some(note)).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    app.open_file(other).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    let covered = app.active_pane;

    app.host_open_wait_files(vec![merge], true, &HashSet::new())
        .unwrap();
    // The person puts the session in front of the other pane themselves, so
    // the covered pane is no longer the one that owes it a return.
    let elsewhere = *app.panes.keys().find(|pane| **pane != covered).unwrap();
    app.activate_pane(elsewhere);
    app.show_terminal(terminal);
    app.activate_pane(covered);
    app.execute_command("close").unwrap();
    assert_eq!(app.terminal_of_pane(elsewhere), Some(terminal));
    // Falling back is not navigating, so it does not go through the ask that
    // ends a detour; the pane reaches another buffer with its claim intact.
    let fallback = app.active().buffer;

    // That pane then leaves the terminal, freeing the session again. Retiring
    // the unrelated buffer the covered pane fell back to must not adopt it.
    app.activate_pane(elsewhere);
    app.leave_terminal();
    app.activate_pane(covered);
    assert_eq!(app.active().buffer, fallback);
    app.execute_command("close").unwrap();

    assert_eq!(
        app.terminal_of_pane(covered),
        None,
        "a spent claim revealed the terminal under an unrelated buffer"
    );
    fs::remove_dir_all(directory).unwrap();
}
