// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use runyte::{
    app::App,
    buffer::Buffer,
    command::Mode,
    config::Config,
    fs_plan::{FsOperation, TransferMode},
    input::{KeyCode, KeyStroke, Modifiers},
    selection::{Range, Selection},
    text::{Change, Transaction},
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "runyte-directory-buffer-{label}-{}-{number}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_directory_renders_as_editable_text_with_directory_markers() {
    let directory = TempDir::new("render");
    fs::write(directory.path().join("file.txt"), "text").unwrap();
    fs::create_dir(directory.path().join("nested")).unwrap();

    let buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert!(buffer.is_directory());
    assert_eq!(buffer.to_string(), "file.txt\nnested/\n");
    assert!(!buffer.directory_row_is_directory(0));
    assert!(buffer.directory_row_is_directory(1));
    assert!(!buffer.is_read_only());
}

#[cfg(unix)]
#[test]
fn a_directory_with_a_newline_filename_is_refused_before_rendering() {
    let directory = TempDir::new("newline-name");
    fs::write(directory.path().join("a\nb"), "text").unwrap();

    let error = Buffer::open_directory(directory.path(), true).unwrap_err();

    assert!(
        error.to_string().contains(
            "filename with control characters that the editable directory explorer cannot represent"
        ),
        "{error:#}"
    );
    assert!(directory.path().join("a\nb").is_file());
    assert!(!directory.path().join("a").exists());
    assert!(!directory.path().join("b").exists());
}

#[test]
fn a_directory_with_a_trailing_whitespace_filename_is_refused_before_rendering() {
    let directory = TempDir::new("trailing-whitespace-filename");
    fs::write(directory.path().join("ambiguous "), "original").unwrap();

    let error = Buffer::open_directory(directory.path(), true)
        .unwrap_err()
        .to_string();

    assert!(error.contains("filename ending in whitespace"), "{error}");
    assert_eq!(
        fs::read_to_string(directory.path().join("ambiguous ")).unwrap(),
        "original"
    );
    assert!(!directory.path().join("ambiguous").exists());
}

#[test]
fn an_edited_control_character_name_is_rejected_before_confirmation() {
    let directory = TempDir::new("typed-control-filename");
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert!(app.buffers[0].apply(&Transaction::insert(0, "bad\tname\n")));

    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();

    assert!(app.status_error);
    assert!(app.status.contains("control characters"), "{}", app.status);
    assert!(app.fs_confirmation.is_none());
    assert!(!directory.path().join("bad\tname").exists());
}

#[test]
fn directory_row_kinds_survive_edits_and_transfers_while_new_rows_use_the_marker() {
    let source = TempDir::new("row-kind-source");
    fs::create_dir(source.path().join("nested")).unwrap();
    let mut source_buffer = Buffer::open_directory(source.path(), true).unwrap();

    assert!(source_buffer.apply(&Transaction::new(vec![Change::new(
        0,
        "nested/".len(),
        "renamed"
    )])));
    assert!(source_buffer.directory_row_is_directory(0));

    let transfer_source = TempDir::new("row-kind-transfer-source");
    fs::create_dir(transfer_source.path().join("moved")).unwrap();
    let transfer_buffer = Buffer::open_directory(transfer_source.path(), true).unwrap();
    let transfer = transfer_buffer.directory_transfer_at(0).unwrap().unwrap();
    let target = TempDir::new("row-kind-target");
    let mut target_buffer = Buffer::open_directory(target.path(), true).unwrap();
    assert!(target_buffer.apply(&Transaction::insert(0, "moved/\n")));
    target_buffer
        .assign_directory_transfers(0, &[transfer], TransferMode::Copy)
        .unwrap();
    assert!(target_buffer.apply(&Transaction::new(vec![Change::new(
        0,
        "moved/".len(),
        "copied"
    )])));
    assert!(target_buffer.directory_row_is_directory(0));

    assert!(target_buffer.apply(&Transaction::insert(
        target_buffer.len_chars(),
        "created/\nplain\n"
    )));
    assert!(target_buffer.directory_row_is_directory(1));
    assert!(!target_buffer.directory_row_is_directory(2));
}

#[test]
fn only_pasted_cuts_are_reported_as_pending_move_sources() {
    let source = TempDir::new("pending-move-source");
    fs::write(source.path().join("note.txt"), "text").unwrap();
    let source_buffer = Buffer::open_directory(source.path(), true).unwrap();
    let transfer = source_buffer.directory_transfer_at(0).unwrap().unwrap();
    let destination = TempDir::new("pending-move-destination");
    let mut destination_buffer = Buffer::open_directory(destination.path(), true).unwrap();

    assert!(destination_buffer.apply(&Transaction::insert(0, "note.txt\n")));
    destination_buffer
        .assign_directory_transfers(0, std::slice::from_ref(&transfer), TransferMode::Copy)
        .unwrap();
    assert!(
        destination_buffer
            .pending_directory_move_sources()
            .is_empty()
    );

    destination_buffer
        .assign_directory_transfers(0, &[transfer], TransferMode::Move)
        .unwrap();
    assert_eq!(
        destination_buffer.pending_directory_move_sources(),
        [source.path().join("note.txt")].into_iter().collect()
    );

    assert!(destination_buffer.apply(&Transaction::delete(0, destination_buffer.len_chars())));
    assert!(
        destination_buffer
            .pending_directory_move_sources()
            .is_empty(),
        "removing the pasted row cancels the pending destination move"
    );
}

#[test]
fn a_multi_entry_edit_is_one_undoable_transaction() {
    let directory = TempDir::new("multi");
    fs::write(directory.path().join("a"), "a").unwrap();
    fs::write(directory.path().join("b"), "b").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert!(buffer.apply(&Transaction::new(vec![
        Change::new(0, 1, "x"),
        Change::new(2, 3, "y"),
    ])));

    assert_eq!(buffer.history_len(), 1);
    assert_eq!(buffer.to_string(), "x\ny\n");
    let plan = buffer.directory_plan().unwrap();
    assert_eq!(plan.operations().len(), 2);
    assert!(
        plan.operations()
            .iter()
            .all(|operation| matches!(operation, FsOperation::Rename { .. }))
    );
    assert!(buffer.undo());
    assert_eq!(buffer.to_string(), "a\nb\n");
    assert!(buffer.directory_plan().unwrap().is_empty());
}

#[test]
fn directory_buffers_use_the_normal_multi_cursor_editor_path() {
    let directory = TempDir::new("modal");
    fs::write(directory.path().join("a"), "a").unwrap();
    fs::write(directory.path().join("b"), "b").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::point(0), Range::point(2)], 0);

    for code in [KeyCode::Char('i'), KeyCode::Char('x'), KeyCode::Escape] {
        app.handle_key(KeyStroke::new(code, Modifiers::NONE))
            .unwrap();
    }

    assert_eq!(app.buffers[0].to_string(), "xa\nxb\n");
    assert_eq!(app.buffers[0].history_len(), 1);
    assert_eq!(
        app.buffers[0].directory_plan().unwrap().operations().len(),
        2
    );
}

#[test]
fn one_escape_leaves_insert_mode_when_path_completion_is_open_in_an_explorer() {
    let directory = TempDir::new("path-completion-escape");
    fs::create_dir_all(directory.path().join("dir_a")).unwrap();
    fs::write(directory.path().join("dir_a/some_existing"), "text").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();

    for code in [KeyCode::Char('A'), KeyCode::Char('s')] {
        app.handle_key(KeyStroke::new(code, Modifiers::NONE))
            .unwrap();
    }

    assert_eq!(app.mode, Mode::Insert);
    assert!(
        app.completion.is_some(),
        "typing after a directory marker should have opened path completion"
    );

    app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();

    assert_eq!(app.mode, Mode::Normal);
    assert!(app.completion.is_none());
}

#[test]
fn writing_a_directory_inside_itself_is_rejected_before_confirmation() {
    let directory = TempDir::new("self-nested-directory");
    fs::create_dir(directory.path().join("dir_a")).unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert!(app.buffers[0].apply(&Transaction::insert("dir_a/".len(), "some_string")));

    for code in [KeyCode::Char(':'), KeyCode::Char('w'), KeyCode::Enter] {
        app.handle_key(KeyStroke::new(code, Modifiers::NONE))
            .unwrap();
    }

    assert!(app.status_error);
    assert!(app.status.contains("cannot move dir_a inside itself"));
    assert!(app.fs_confirmation.is_none());
    assert!(directory.path().join("dir_a").is_dir());
    assert!(!directory.path().join("dir_a/some_string").exists());
}

#[test]
fn cut_and_paste_reordering_keeps_hidden_entry_identities() {
    let directory = TempDir::new("identity");
    fs::write(directory.path().join("a"), "a").unwrap();
    fs::write(directory.path().join("b"), "b").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert!(buffer.apply(&Transaction::delete(0, 2)));
    assert_eq!(buffer.to_string(), "b\n");
    assert!(buffer.apply(&Transaction::insert(2, "a\n")));
    assert_eq!(buffer.to_string(), "b\na\n");
    assert!(
        buffer.directory_plan().unwrap().is_empty(),
        "moving rows must not become delete-plus-create"
    );

    assert!(buffer.apply(&Transaction::change(0, 1, "c")));
    let plan = buffer.directory_plan().unwrap();
    assert!(matches!(
        plan.operations(),
        [FsOperation::Rename { from, to, .. }]
            if from == Path::new("b") && to == Path::new("c")
    ));
}

#[test]
fn deleting_an_entry_does_not_turn_an_existing_new_row_into_a_rename() {
    let directory = TempDir::new("create-then-delete");
    fs::write(directory.path().join("removed"), "original contents").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert!(buffer.apply(&Transaction::insert(buffer.len_chars(), "created\n")));
    assert!(buffer.apply(&Transaction::delete(0, "removed\n".len())));

    let plan = buffer.directory_plan().unwrap();
    assert!(matches!(
        plan.operations(),
        [FsOperation::Create { path: created, .. }, FsOperation::Delete { path: removed, .. }]
            if created == Path::new("created") && removed == Path::new("removed")
    ));
}

#[test]
fn reordering_all_rows_in_one_transaction_keeps_their_identities() {
    let directory = TempDir::new("single-transaction-reorder");
    fs::write(directory.path().join("a"), "from a").unwrap();
    fs::write(directory.path().join("b"), "from b").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert!(buffer.apply(&Transaction::change(0, buffer.len_chars(), "b\na\n")));

    assert!(buffer.directory_plan().unwrap().is_empty());
}

#[test]
fn pasting_and_repathing_an_entry_produces_a_copy() {
    let directory = TempDir::new("copy-identity");
    fs::write(directory.path().join("source"), "content").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert!(buffer.apply(&Transaction::insert("source\n".len(), "source\n")));
    assert_eq!(buffer.to_string(), "source\nsource\n");
    assert!(buffer.apply(&Transaction::change(
        "source\n".len(),
        "source\nsource".len(),
        "../destination/copied",
    )));

    let plan = buffer.directory_plan().unwrap();
    assert!(matches!(
        plan.operations(),
        [FsOperation::Copy { from, to, .. }]
            if from == Path::new("source")
                && to == Path::new("../destination/copied")
    ));
}

#[test]
fn navigation_never_applies_an_edited_entry_name() {
    let directory = TempDir::new("navigation");
    fs::write(directory.path().join("before"), "text").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert!(buffer.apply(&Transaction::change(0, "before".len(), "after")));
    let error = buffer.directory_entry_path(0).unwrap_err().to_string();

    assert!(error.contains("save directory edits before opening"));
    assert!(directory.path().join("before").is_file());
    assert!(!directory.path().join("after").exists());
}

#[test]
fn hidden_entries_are_listed_only_on_request_and_are_never_planned_as_deletions() {
    let directory = TempDir::new("hidden");
    fs::write(directory.path().join(".env"), "secret").unwrap();
    fs::write(directory.path().join("visible.txt"), "text").unwrap();

    let shown = Buffer::open_directory(directory.path(), true).unwrap();
    assert_eq!(shown.to_string(), ".env\nvisible.txt\n");

    let mut listing = Buffer::open_directory(directory.path(), false).unwrap();
    assert_eq!(listing.to_string(), "visible.txt\n");
    assert!(listing.apply(&Transaction::change(0, "visible.txt".len(), "renamed.txt")));

    let plan = listing.directory_plan().unwrap();
    assert!(
        matches!(
            plan.operations(),
            [FsOperation::Rename { from, to, .. }]
                if from == Path::new("visible.txt") && to == Path::new("renamed.txt")
        ),
        "an unlisted dotfile must not become a deletion: {:?}",
        plan.operations()
    );
}

#[test]
fn applying_a_plan_from_a_listing_without_dotfiles_leaves_them_on_disk() {
    let directory = TempDir::new("hidden-apply");
    fs::write(directory.path().join(".env"), "secret").unwrap();
    fs::write(directory.path().join("gone"), "text").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert_eq!(app.buffers[0].to_string(), "gone\n");

    assert!(app.buffers[0].apply(&Transaction::delete(0, "gone\n".len())));
    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();
    // Use permanent deletion in the fixture so the test does not depend on
    // access to the person's platform trash directory.
    app.handle_key(KeyStroke::new(KeyCode::Char('P'), Modifiers::NONE))
        .unwrap();

    assert!(!directory.path().join("gone").exists());
    assert!(
        directory.path().join(".env").is_file(),
        "a dotfile the listing never showed must survive its plan"
    );
}

#[test]
fn the_dot_key_shows_and_hides_dotfiles_in_the_explorer() {
    let directory = TempDir::new("hidden-toggle");
    fs::write(directory.path().join(".env"), "secret").unwrap();
    fs::write(directory.path().join("visible.txt"), "text").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert_eq!(app.buffers[0].to_string(), "visible.txt\n");

    app.handle_key(KeyStroke::new(KeyCode::Char('.'), Modifiers::NONE))
        .unwrap();

    assert_eq!(app.buffers[0].to_string(), ".env\nvisible.txt\n");
    assert!(app.config.editor.show_hidden_files);
    assert_eq!(app.status, "showing hidden files");
    assert!(!app.status_error);

    app.handle_key(KeyStroke::new(KeyCode::Char('.'), Modifiers::NONE))
        .unwrap();

    assert_eq!(app.buffers[0].to_string(), "visible.txt\n");
    assert!(!app.config.editor.show_hidden_files);
    assert_eq!(app.status, "hiding hidden files");
}

#[test]
fn showing_dotfiles_reloads_every_clean_explorer_rather_than_the_active_pane_alone() {
    let directory = TempDir::new("hidden-panes");
    fs::create_dir(directory.path().join("nested")).unwrap();
    fs::write(directory.path().join(".root-secret"), "root").unwrap();
    fs::write(
        directory.path().join("nested").join(".nested-secret"),
        "nested",
    )
    .unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert_eq!(app.buffers[0].to_string(), "nested/\n");

    // The split takes an explorer of its own as soon as it opens `nested/`,
    // leaving two listings that the preference has to reach.
    for code in [KeyCode::Char(' '), KeyCode::Char('w'), KeyCode::Char('v')] {
        app.handle_key(KeyStroke::new(code, Modifiers::NONE))
            .unwrap();
    }
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();
    assert_eq!(app.buffers.len(), 2);
    assert_eq!(app.buffers[1].to_string(), "");

    app.handle_key(KeyStroke::new(KeyCode::Char('.'), Modifiers::NONE))
        .unwrap();

    assert_eq!(app.buffers[1].to_string(), ".nested-secret\n");
    assert_eq!(app.buffers[0].to_string(), ".root-secret\nnested/\n");
}

#[test]
fn showing_dotfiles_is_refused_while_the_explorer_has_unsaved_edits() {
    let directory = TempDir::new("hidden-dirty");
    fs::write(directory.path().join(".env"), "secret").unwrap();
    fs::write(directory.path().join("visible.txt"), "text").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert!(app.buffers[0].apply(&Transaction::insert(0, "created\n")));

    app.handle_key(KeyStroke::new(KeyCode::Char('.'), Modifiers::NONE))
        .unwrap();

    assert!(app.status_error);
    assert!(app.status.contains("unsaved edits"), "{}", app.status);
    assert!(!app.config.editor.show_hidden_files);
    assert_eq!(app.buffers[0].to_string(), "created\nvisible.txt\n");
}

#[test]
fn writing_only_opens_confirmation_and_enter_applies_the_plan() {
    let directory = TempDir::new("confirm");
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert!(app.buffers[0].apply(&Transaction::insert(0, "created\n")));

    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();

    assert!(app.fs_confirmation.is_some());
    assert!(
        !directory.path().join("created").exists(),
        "saving a directory buffer must only prepare a plan"
    );

    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert!(app.fs_confirmation.is_none());
    assert!(directory.path().join("created").is_file());
    assert!(!app.buffers[0].dirty);
}

#[test]
fn applying_a_plan_keeps_the_edited_order_until_the_explorer_is_reentered() {
    let directory = TempDir::new("saved-order");
    fs::write(directory.path().join("alpha"), "alpha").unwrap();
    fs::write(directory.path().join("middle"), "middle").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert_eq!(app.buffers[0].to_string(), "alpha\nmiddle\n");

    assert!(app.buffers[0].apply(&Transaction::change(0, "alpha".len(), "zulu")));
    let end = app.buffers[0].len_chars();
    assert!(app.buffers[0].apply(&Transaction::insert(end, "aardvark\n")));
    assert_eq!(app.buffers[0].to_string(), "zulu\nmiddle\naardvark\n");

    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert_eq!(app.buffers[0].to_string(), "zulu\nmiddle\naardvark\n");
    assert!(!app.buffers[0].dirty);
    assert!(directory.path().join("zulu").is_file());
    assert!(directory.path().join("aardvark").is_file());
    assert!(app.buffers[0].directory_plan().unwrap().is_empty());

    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.path().join("zulu").as_path())
    );
    for code in [KeyCode::Char(' '), KeyCode::Char('e')] {
        app.handle_key(KeyStroke::new(code, Modifiers::NONE))
            .unwrap();
    }

    assert!(app.active_buffer().is_directory());
    assert_eq!(app.active_buffer().to_string(), "aardvark\nmiddle\nzulu\n");
}

#[test]
fn writing_whitespace_only_explorer_edits_refreshes_the_listing() {
    let directory = TempDir::new("whitespace-only");
    fs::create_dir(directory.path().join("context")).unwrap();
    fs::write(directory.path().join("note.txt"), "text").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    let canonical = "context/\nnote.txt\n";
    assert_eq!(app.buffers[0].to_string(), canonical);

    assert!(app.buffers[0].apply(&Transaction::new(vec![
        Change::new("context/".len(), "context/".len(), "  "),
        Change::new(canonical.len() - 1, canonical.len() - 1, "  "),
        Change::new(canonical.len(), canonical.len(), " \n\n\t"),
    ])));
    assert!(app.buffers[0].dirty);
    assert!(app.buffers[0].directory_plan().unwrap().is_empty());

    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();

    assert!(app.fs_confirmation.is_none());
    assert_eq!(app.buffers[0].to_string(), canonical);
    assert!(!app.buffers[0].dirty);
    assert_eq!(app.status, "directory has no filesystem changes");
    assert!(!app.status_error);
}

#[cfg(unix)]
#[test]
fn a_symlink_is_annotated_with_its_target_without_that_hint_entering_the_text() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new("symlink-hint");
    fs::write(directory.path().join("true_file.txt"), "text").unwrap();
    fs::create_dir(directory.path().join("nested")).unwrap();
    symlink("true_file.txt", directory.path().join("file.txt")).unwrap();
    symlink("nested", directory.path().join("shortcut")).unwrap();

    let buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert_eq!(
        buffer.to_string(),
        "file.txt\nnested/\nshortcut\ntrue_file.txt\n",
        "a hint is not part of the editable listing"
    );
    let hints = buffer.row_hints();
    assert_eq!(hints.text(0), Some("→ true_file.txt"));
    assert_eq!(hints.text(2), Some("→ nested"));
    assert_eq!(hints.text(1), None);
    assert_eq!(hints.text(3), None);
    // Every hint in one listing starts in the same column, two cells past the
    // longest row that carries one.
    assert_eq!(hints.column(), "shortcut".len() + 2);
    assert_eq!(
        hints.rendered(0, "file.txt".len(), 40).as_deref(),
        Some("  → true_file.txt")
    );
    assert_eq!(
        hints.rendered(2, "shortcut".len(), 40).as_deref(),
        Some("  → nested")
    );
    assert!(!buffer.directory_row_is_directory(0));
    assert!(buffer.directory_plan().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn opening_a_symlink_resolves_it_to_the_file_it_points_at() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new("symlink-open");
    let target = directory.path().join("true_file.txt");
    fs::write(&target, "text\n").unwrap();
    symlink("true_file.txt", directory.path().join("file.txt")).unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert_eq!(app.buffers[0].to_string(), "file.txt\ntrue_file.txt\n");

    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(target.canonicalize().unwrap().as_path()),
        "a link must open the file Git and the language server know about"
    );
    assert_eq!(app.active_buffer().to_string(), "text\n");
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_opens_the_directory_it_points_at() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new("symlink-directory");
    fs::create_dir(directory.path().join("nested")).unwrap();
    fs::write(directory.path().join("nested/inside.txt"), "text").unwrap();
    symlink("nested", directory.path().join("shortcut")).unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    let shortcut = app.buffers[0].line_to_offset(1);
    app.panes.get_mut(&0).unwrap().selection = Selection::point(shortcut);

    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert!(app.active_buffer().is_directory());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(
            directory
                .path()
                .join("nested")
                .canonicalize()
                .unwrap()
                .as_path()
        )
    );
    assert_eq!(app.active_buffer().to_string(), "inside.txt\n");
}

#[cfg(unix)]
#[test]
fn renaming_and_deleting_symlinks_works_on_the_links_themselves() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new("symlink-edit");
    fs::write(directory.path().join("true_file.txt"), "text").unwrap();
    symlink("true_file.txt", directory.path().join("file.txt")).unwrap();
    symlink("true_file.txt", directory.path().join("spare.txt")).unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), true).unwrap();
    assert_eq!(buffer.to_string(), "file.txt\nspare.txt\ntrue_file.txt\n");

    assert!(buffer.apply(&Transaction::change(0, "file.txt".len(), "renamed.txt")));
    let removed = buffer.line_to_offset(1);
    assert!(buffer.apply(&Transaction::delete(removed, removed + "spare.txt\n".len())));
    assert_eq!(buffer.to_string(), "renamed.txt\ntrue_file.txt\n");
    // A pending rename does not detach the row from the link it names.
    assert_eq!(buffer.row_hints().text(0), Some("→ true_file.txt"));

    let plan = buffer.directory_plan().unwrap();
    assert!(
        matches!(
            plan.operations(),
            [
                FsOperation::Rename { from, to, .. },
                FsOperation::Delete { path, .. }
            ] if from == Path::new("file.txt")
                && to == Path::new("renamed.txt")
                && path == Path::new("spare.txt")
        ),
        "{:?}",
        plan.operations()
    );
    plan.apply(runyte::fs_plan::DeletionMode::Permanent)
        .unwrap();

    assert!(
        fs::symlink_metadata(directory.path().join("renamed.txt"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "renaming a link must not replace it with its target"
    );
    assert!(!directory.path().join("spare.txt").exists());
    assert!(directory.path().join("true_file.txt").is_file());
}

#[cfg(unix)]
#[test]
fn a_broken_symlink_is_listed_and_reports_why_it_cannot_be_opened() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new("symlink-broken");
    symlink("missing.txt", directory.path().join("dangling.txt")).unwrap();
    let buffer = Buffer::open_directory(directory.path(), true).unwrap();

    assert_eq!(buffer.to_string(), "dangling.txt\n");
    assert_eq!(buffer.row_hints().text(0), Some("→ missing.txt"));
    let error = buffer.directory_entry_path(0).unwrap_err().to_string();

    assert!(error.contains("broken symlink"), "{error}");
}
