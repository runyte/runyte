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
    config::{Config, ExplorerSort},
    directory_buffer::ListingView,
    fs_plan::{FsOperation, TransferMode, TrashBackend},
    input::{KeyCode, KeyStroke, Modifiers},
    selection::{Range, Selection},
    text::{Change, Transaction},
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

/// The default view, listing dotfiles: most of these tests are about what a
/// plan does with the rows rather than about which rows are listed.
fn listing(show_hidden: bool) -> ListingView {
    ListingView {
        show_hidden,
        ..ListingView::default()
    }
}

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

struct TemporaryTrash {
    destination: PathBuf,
    fail: bool,
}

impl TrashBackend for TemporaryTrash {
    fn delete(&self, path: &Path) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("injected trash failure");
        }
        let name = path.file_name().expect("a trashed fixture has a name");
        fs::rename(path, self.destination.join(name))?;
        Ok(())
    }
}

/// The four fixture entries are deliberately ordered differently by each key:
/// by name `alpha.txt` leads, by size `tiny.txt` does, and by modification
/// time `old.txt` does. A directory is present in every case so its grouping
/// is pinned alongside.
fn sorting_fixture(label: &str) -> TempDir {
    let directory = TempDir::new(label);
    fs::create_dir(directory.path().join("zed")).unwrap();
    fs::write(directory.path().join("alpha.txt"), vec![b'a'; 300]).unwrap();
    fs::write(directory.path().join("old.txt"), vec![b'o'; 200]).unwrap();
    fs::write(directory.path().join("tiny.txt"), b"t").unwrap();
    let epoch = std::time::SystemTime::UNIX_EPOCH;
    let age = |name: &str, seconds: u64| {
        fs::File::options()
            .write(true)
            .open(directory.path().join(name))
            .unwrap()
            .set_modified(epoch + std::time::Duration::from_secs(seconds))
            .unwrap();
    };
    age("old.txt", 1_000);
    age("alpha.txt", 2_000);
    age("tiny.txt", 3_000);
    directory
}

fn listed(directory: &TempDir, sort: ExplorerSort) -> String {
    Buffer::open_directory(
        directory.path(),
        ListingView {
            sort,
            ..listing(true)
        },
    )
    .unwrap()
    .to_string()
}

#[test]
fn each_listing_order_sorts_by_its_own_key_with_directories_first() {
    let directory = sorting_fixture("sort-orders");

    assert_eq!(
        listed(&directory, ExplorerSort::Name),
        "zed/\nalpha.txt\nold.txt\ntiny.txt\n"
    );
    assert_eq!(
        listed(&directory, ExplorerSort::NameDescending),
        "zed/\ntiny.txt\nold.txt\nalpha.txt\n"
    );
    // Ascending time is oldest first, which is the opposite of the name order
    // here and so cannot be passing by accident.
    assert_eq!(
        listed(&directory, ExplorerSort::Modified),
        "zed/\nold.txt\nalpha.txt\ntiny.txt\n"
    );
    assert_eq!(
        listed(&directory, ExplorerSort::ModifiedDescending),
        "zed/\ntiny.txt\nalpha.txt\nold.txt\n"
    );
    assert_eq!(
        listed(&directory, ExplorerSort::Size),
        "zed/\ntiny.txt\nold.txt\nalpha.txt\n"
    );
    assert_eq!(
        listed(&directory, ExplorerSort::SizeDescending),
        "zed/\nalpha.txt\nold.txt\ntiny.txt\n"
    );
}

/// A directory's own length is its bookkeeping rather than what it holds, so a
/// size order has nothing to say about one. Directories keep their name order
/// under either direction, while the files around them reverse.
#[test]
fn a_size_order_leaves_directories_in_name_order() {
    let directory = TempDir::new("sort-directories");
    for name in ["beta", "alpha"] {
        fs::create_dir(directory.path().join(name)).unwrap();
    }
    fs::write(directory.path().join("big.txt"), vec![b'b'; 500]).unwrap();
    fs::write(directory.path().join("small.txt"), b"s").unwrap();

    assert_eq!(
        listed(&directory, ExplorerSort::Size),
        "alpha/\nbeta/\nsmall.txt\nbig.txt\n"
    );
    assert_eq!(
        listed(&directory, ExplorerSort::SizeDescending),
        "alpha/\nbeta/\nbig.txt\nsmall.txt\n"
    );
}

/// Equal keys fall back to the name, so an order never leaves two rows in an
/// arbitrary relative position that a redraw could change.
#[test]
fn entries_sharing_a_key_keep_their_name_order() {
    let directory = TempDir::new("sort-ties");
    for name in ["c.txt", "a.txt", "b.txt"] {
        fs::write(directory.path().join(name), b"x").unwrap();
    }

    assert_eq!(
        listed(&directory, ExplorerSort::Size),
        "a.txt\nb.txt\nc.txt\n"
    );
    assert_eq!(
        listed(&directory, ExplorerSort::SizeDescending),
        "a.txt\nb.txt\nc.txt\n"
    );
}

/// Reordering rows is a change of projection, not of identity: the plan built
/// from a listing must not read a sorted row as a different filesystem entry.
#[test]
fn a_listing_order_does_not_turn_a_rename_into_a_delete_and_create() {
    let directory = sorting_fixture("sort-identity");
    let mut buffer = Buffer::open_directory(
        directory.path(),
        ListingView {
            sort: ExplorerSort::SizeDescending,
            ..listing(true)
        },
    )
    .unwrap();
    assert_eq!(buffer.to_string(), "zed/\nalpha.txt\nold.txt\ntiny.txt\n");

    // `alpha.txt` sits on the second row only because it is the largest file.
    let start = "zed/\n".len();
    assert!(buffer.apply(&Transaction::change(
        start,
        start + "alpha.txt".len(),
        "renamed.txt",
    )));

    let plan = buffer.directory_plan().unwrap();
    assert_eq!(plan.operations().len(), 1, "{:?}", plan.operations());
    assert!(
        matches!(
            plan.operations().first(),
            Some(FsOperation::Rename { from, to, .. })
                if from.ends_with("alpha.txt") && to.ends_with("renamed.txt")
        ),
        "{:?}",
        plan.operations()
    );
}

/// The order is a setting like the other two: choosing it from the explorer's
/// own list saves it and re-projects every open listing.
#[test]
fn choosing_a_listing_order_saves_it_and_reprojects_the_explorer() {
    let directory = sorting_fixture("sort-choice");
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    let settings = TempDir::new("sort-choice-config");
    let config_path = settings.path().join("config.yaml");
    fs::write(&config_path, "editor:\n  explorer_sort: name\n").unwrap();
    app.note_loaded_config(&config_path);
    assert_eq!(
        app.buffers[0].to_string(),
        "zed/\nalpha.txt\nold.txt\ntiny.txt\n"
    );

    // Tab opens the explorer's contextual actions; `o` is the listing order.
    app.handle_key(KeyStroke::new(KeyCode::Tab, Modifiers::NONE))
        .unwrap();
    app.handle_key(KeyStroke::char('o')).unwrap();
    let rows = app
        .list
        .as_ref()
        .expect("the order list is open")
        .items
        .iter()
        .map(|item| format!("{} {}", item.label, item.detail))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), ExplorerSort::ALL.len());
    assert_eq!(rows[0], "name, A to Z in use");
    assert_eq!(rows[5], "size, largest first choice");

    // Typed characters filter the list, so the selection moves on the arrows.
    for _ in 0..5 {
        app.handle_key(KeyStroke::new(KeyCode::Down, Modifiers::NONE))
            .unwrap();
    }
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert_eq!(
        app.config.editor.explorer_sort,
        ExplorerSort::SizeDescending
    );
    assert_eq!(
        app.buffers[0].to_string(),
        "zed/\nalpha.txt\nold.txt\ntiny.txt\n"
    );
    assert!(
        fs::read_to_string(&config_path)
            .unwrap()
            .contains("explorer_sort: size_descending"),
        "the chosen order must reach the configuration file"
    );
}

/// Re-reading a listing would discard rows that have not been written yet, so
/// the two orders that re-project are refused while an explorer is modified.
/// Details only prefix the rows already there, so they stay available.
#[test]
fn a_modified_explorer_refuses_a_reprojection_but_still_shows_details() {
    let directory = TempDir::new("sort-dirty");
    fs::write(directory.path().join("visible.txt"), "text").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    assert!(app.buffers[0].apply(&Transaction::insert(0, "created\n")));

    app.handle_key(KeyStroke::new(KeyCode::Tab, Modifiers::NONE))
        .unwrap();
    app.handle_key(KeyStroke::char('o')).unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Down, Modifiers::NONE))
        .unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert!(app.status_error, "{}", app.status);
    assert!(app.status.contains("unsaved edits"), "{}", app.status);
    assert_eq!(app.config.editor.explorer_sort, ExplorerSort::Name);
    assert_eq!(app.buffers[0].to_string(), "created\nvisible.txt\n");

    app.handle_key(KeyStroke::char('?')).unwrap();

    assert!(app.active_buffer().directory_details_shown());
    assert_eq!(app.buffers[0].to_string(), "created\nvisible.txt\n");
}

#[test]
fn a_directory_renders_as_editable_text_with_directory_markers() {
    let directory = TempDir::new("render");
    fs::write(directory.path().join("file.txt"), "text").unwrap();
    fs::create_dir(directory.path().join("nested")).unwrap();

    let buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

    assert!(buffer.is_directory());
    assert_eq!(buffer.to_string(), "nested/\nfile.txt\n");
    assert!(buffer.directory_row_is_directory(0));
    assert!(!buffer.directory_row_is_directory(1));
    assert!(!buffer.is_read_only());
}

#[cfg(unix)]
#[test]
fn a_directory_with_a_newline_filename_is_refused_before_rendering() {
    let directory = TempDir::new("newline-name");
    fs::write(directory.path().join("a\nb"), "text").unwrap();

    let error = Buffer::open_directory(directory.path(), listing(true)).unwrap_err();

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

    let error = Buffer::open_directory(directory.path(), listing(true))
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
    let mut source_buffer = Buffer::open_directory(source.path(), listing(true)).unwrap();

    assert!(source_buffer.apply(&Transaction::new(vec![Change::new(
        0,
        "nested/".len(),
        "renamed"
    )])));
    assert!(source_buffer.directory_row_is_directory(0));

    let transfer_source = TempDir::new("row-kind-transfer-source");
    fs::create_dir(transfer_source.path().join("moved")).unwrap();
    let transfer_buffer = Buffer::open_directory(transfer_source.path(), listing(true)).unwrap();
    let transfer = transfer_buffer.directory_transfer_at(0).unwrap().unwrap();
    let target = TempDir::new("row-kind-target");
    let mut target_buffer = Buffer::open_directory(target.path(), listing(true)).unwrap();
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
    let source_buffer = Buffer::open_directory(source.path(), listing(true)).unwrap();
    let transfer = source_buffer.directory_transfer_at(0).unwrap().unwrap();
    let destination = TempDir::new("pending-move-destination");
    let mut destination_buffer = Buffer::open_directory(destination.path(), listing(true)).unwrap();

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
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

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
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

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
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

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
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

    assert!(buffer.apply(&Transaction::change(0, buffer.len_chars(), "b\na\n")));

    assert!(buffer.directory_plan().unwrap().is_empty());
}

#[test]
fn pasting_and_repathing_an_entry_produces_a_copy() {
    let directory = TempDir::new("copy-identity");
    fs::write(directory.path().join("source"), "content").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

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
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

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

    let shown = Buffer::open_directory(directory.path(), listing(true)).unwrap();
    assert_eq!(shown.to_string(), ".env\nvisible.txt\n");

    let mut listing = Buffer::open_directory(directory.path(), listing(false)).unwrap();
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
    // The toggle writes the setting, so it must be pointed at a configuration
    // file of this test's own rather than the one this machine belongs to. It
    // lives outside the listed directory, which the explorer is showing.
    let settings = TempDir::new("hidden-toggle-config");
    let config_path = settings.path().join("config.yaml");
    fs::write(&config_path, "editor:\n  show_hidden_files: false\n").unwrap();
    app.note_loaded_config(&config_path);
    assert_eq!(app.buffers[0].to_string(), "visible.txt\n");

    app.handle_key(KeyStroke::new(KeyCode::Char('.'), Modifiers::NONE))
        .unwrap();

    assert_eq!(app.buffers[0].to_string(), ".env\nvisible.txt\n");
    assert!(app.config.editor.show_hidden_files);
    assert_eq!(app.status, "Hidden files: true");
    assert!(!app.status_error);
    assert!(
        fs::read_to_string(&config_path)
            .unwrap()
            .contains("show_hidden_files: true"),
        "the toggle is a setting and must reach the configuration file"
    );

    app.handle_key(KeyStroke::new(KeyCode::Char('.'), Modifiers::NONE))
        .unwrap();

    assert_eq!(app.buffers[0].to_string(), "visible.txt\n");
    assert!(!app.config.editor.show_hidden_files);
    assert_eq!(app.status, "Hidden files: false");
    assert!(
        fs::read_to_string(&config_path)
            .unwrap()
            .contains("show_hidden_files: false")
    );
}

/// A toggle whose value cannot be written still applies: a configuration file
/// that is missing or unpatchable has nothing to do with what the explorer was
/// asked to show.
#[test]
fn a_toggle_that_cannot_be_saved_still_changes_the_listing() {
    let directory = TempDir::new("hidden-unsaved");
    fs::write(directory.path().join(".env"), "secret").unwrap();
    fs::write(directory.path().join("visible.txt"), "text").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();

    app.handle_key(KeyStroke::new(KeyCode::Char('.'), Modifiers::NONE))
        .unwrap();

    assert_eq!(app.buffers[0].to_string(), ".env\nvisible.txt\n");
    assert!(app.config.editor.show_hidden_files);
    assert!(
        app.status.starts_with("Hidden files: true · not saved:"),
        "{}",
        app.status
    );
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
    assert_eq!(app.buffers[0].to_string(), "nested/\n.root-secret\n");
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
fn enter_uses_the_injected_trash_backend_only_after_confirmation() {
    let directory = TempDir::new("trash-confirm");
    let trash = TempDir::new("trash-destination");
    let source = directory.path().join("gone");
    fs::write(&source, "recoverable contents").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    app.set_trash_backend(Box::new(TemporaryTrash {
        destination: trash.path().to_path_buf(),
        fail: false,
    }));
    assert!(app.buffers[0].apply(&Transaction::delete(0, "gone\n".len())));

    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();
    assert!(app.fs_confirmation.is_some());
    assert!(
        source.exists(),
        "preparing the plan must not delete its source"
    );

    app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();
    assert!(app.fs_confirmation.is_none());
    assert!(
        source.exists(),
        "cancelling must leave the source untouched"
    );

    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(trash.path().join("gone")).unwrap(),
        "recoverable contents"
    );
    assert!(!app.buffers[0].dirty);
}

#[test]
fn trash_backend_failure_preserves_the_source_and_explorer_edits() {
    let directory = TempDir::new("trash-failure");
    let trash = TempDir::new("trash-failure-destination");
    let source = directory.path().join("gone");
    fs::write(&source, "still here").unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    app.set_trash_backend(Box::new(TemporaryTrash {
        destination: trash.path().to_path_buf(),
        fail: true,
    }));
    assert!(app.buffers[0].apply(&Transaction::delete(0, "gone\n".len())));

    app.handle_key(KeyStroke::new(KeyCode::Char('s'), Modifiers::CONTROL))
        .unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();

    assert!(source.exists());
    assert_eq!(fs::read_to_string(source).unwrap(), "still here");
    assert!(app.status_error);
    assert!(
        app.status.contains("injected trash failure"),
        "{}",
        app.status
    );
    assert!(
        app.status.contains("directory edits retained"),
        "{}",
        app.status
    );
    assert_eq!(app.buffers[0].to_string(), "");
    assert!(app.buffers[0].dirty);
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

    let buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

    assert_eq!(
        buffer.to_string(),
        "nested/\nfile.txt\nshortcut\ntrue_file.txt\n",
        "a hint is not part of the editable listing"
    );
    let hints = buffer.row_hints();
    assert_eq!(hints.text(1), Some("→ true_file.txt"));
    assert_eq!(hints.text(2), Some("→ nested"));
    assert_eq!(hints.text(0), None);
    assert_eq!(hints.text(3), None);
    // Every hint in one listing starts in the same column, two cells past the
    // longest row that carries one.
    assert_eq!(hints.column(), "shortcut".len() + 2);
    assert_eq!(
        hints.rendered(1, "file.txt".len(), 40).as_deref(),
        Some("  → true_file.txt")
    );
    assert_eq!(
        hints.rendered(2, "shortcut".len(), 40).as_deref(),
        Some("  → nested")
    );
    assert!(!buffer.directory_row_is_directory(1));
    assert!(buffer.directory_plan().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn question_mark_toggles_aligned_file_details_without_editing_the_listing() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = TempDir::new("details-toggle");
    let file = directory.path().join("AGENTS.md");
    fs::write(&file, vec![b'x'; 14 * 1024]).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("AGENTS.md", directory.path().join("CLAUDE.md")).unwrap();
    fs::create_dir(directory.path().join("nested")).unwrap();
    let mut app = App::new(Config::default(), Some(directory.path().to_path_buf())).unwrap();
    let settings = TempDir::new("details-toggle-config");
    let config_path = settings.path().join("config.yaml");
    fs::write(&config_path, "editor:\n  explorer_details: false\n").unwrap();
    app.note_loaded_config(&config_path);
    let editable = app.active_buffer().to_string();

    app.handle_key(KeyStroke::char('?')).unwrap();

    assert_eq!(app.status, "Explorer details: true");
    assert!(
        fs::read_to_string(&config_path)
            .unwrap()
            .contains("explorer_details: true"),
        "the toggle is a setting and must reach the configuration file"
    );
    assert_eq!(app.active_buffer().to_string(), editable);
    assert!(!app.active_buffer().dirty);
    assert!(app.active_buffer().directory_plan().unwrap().is_empty());
    let hints = app.active_buffer().row_hints();
    let file_prefix = hints.prefix(1).expect("the file has a details prefix");
    assert!(file_prefix.starts_with("-rw-r----- "), "{file_prefix:?}");
    assert!(file_prefix.contains(" 14K "), "{file_prefix:?}");
    let columns = file_prefix.split_whitespace().collect::<Vec<_>>();
    assert_eq!(columns.len(), 7, "{file_prefix:?}");
    assert_eq!(columns[3], "14K");
    assert!(columns[6].contains(':'), "{file_prefix:?}");
    // A symlink's own mode bits are not portable — Linux reports 0o777 and
    // macOS 0o755 — so the row asserts the type character and that the details
    // still line up with the other rows.
    let link_prefix = hints.prefix(2).expect("the symlink has a details prefix");
    assert!(link_prefix.starts_with('l'), "{link_prefix:?}");
    assert_eq!(link_prefix.len(), file_prefix.len(), "{link_prefix:?}");
    assert_eq!(hints.text(2), Some("→ AGENTS.md"));

    for key in [KeyStroke::char(' '), KeyStroke::char('r')] {
        app.handle_key(key).unwrap();
    }
    assert!(app.active_buffer().directory_details_shown());
    assert!(app.active_buffer().row_hints().prefix(0).is_some());

    // `nested/` leads the listing: directories group before files.
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();
    assert_eq!(
        app.active_buffer().directory_root(),
        Some(directory.path().join("nested").as_path())
    );
    assert!(app.active_buffer().directory_details_shown());
    app.handle_key(KeyStroke::new(KeyCode::Backspace, Modifiers::NONE))
        .unwrap();
    assert_eq!(app.active_buffer().directory_root(), Some(directory.path()));
    assert!(app.active_buffer().directory_details_shown());

    app.handle_key(KeyStroke::char('?')).unwrap();

    assert_eq!(app.status, "Explorer details: false");
    assert_eq!(app.active_buffer().to_string(), editable);
    assert_eq!(app.active_buffer().row_hints().prefix(0), None);
    assert_eq!(app.active_buffer().row_hints().text(2), Some("→ AGENTS.md"));
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
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();
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
    let buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

    assert_eq!(buffer.to_string(), "dangling.txt\n");
    assert_eq!(buffer.row_hints().text(0), Some("→ missing.txt"));
    let error = buffer.directory_entry_path(0).unwrap_err().to_string();

    assert!(error.contains("broken symlink"), "{error}");
}

/// Copying a row reads it from the listing's own identities, so a row that has
/// no identity yet, or whose name no longer matches the one on disk, has to be
/// written before it can be the source of anything.
#[test]
fn a_row_can_only_be_copied_once_the_listing_and_the_disk_agree_about_it() {
    let directory = TempDir::new("transfer-source-state");
    fs::write(directory.path().join("note.txt"), "text").unwrap();
    let mut buffer = Buffer::open_directory(directory.path(), listing(true)).unwrap();

    let transfer = buffer.directory_transfer_at(0).unwrap().unwrap();
    assert_eq!(transfer.source, directory.path().join("note.txt"));
    assert_eq!(transfer.label, "note.txt");

    assert!(
        buffer.directory_transfer_at(9).unwrap().is_none(),
        "a row past the end of the listing is nothing to copy"
    );

    let end = buffer.len_chars();
    assert!(buffer.apply(&Transaction::insert(end, "created\n")));
    let error = buffer.directory_transfer_at(1).unwrap_err().to_string();
    assert!(error.contains("write it before copying it"), "{error}");

    assert!(buffer.apply(&Transaction::new(vec![Change::new(
        0,
        "note.txt".len(),
        "renamed.txt"
    )])));
    let error = buffer.directory_transfer_at(0).unwrap_err().to_string();
    assert!(error.contains("pending edits"), "{error}");
}

/// A row that is itself a pending transfer can be copied again: what it names
/// is the transfer's own source, not a path inside the listing showing it.
#[test]
fn a_pasted_row_is_copied_from_the_source_it_still_points_at() {
    let source = TempDir::new("second-hand-source");
    fs::create_dir(source.path().join("tree")).unwrap();
    let source_buffer = Buffer::open_directory(source.path(), listing(true)).unwrap();
    let transfer = source_buffer.directory_transfer_at(0).unwrap().unwrap();

    let destination = TempDir::new("second-hand-destination");
    let mut destination_buffer = Buffer::open_directory(destination.path(), listing(true)).unwrap();
    assert!(destination_buffer.apply(&Transaction::insert(0, "tree/\n")));
    destination_buffer
        .assign_directory_transfers(0, &[transfer], TransferMode::Copy)
        .unwrap();

    let again = destination_buffer
        .directory_transfer_at(0)
        .unwrap()
        .unwrap();
    assert_eq!(
        again.source,
        source.path().join("tree"),
        "the second copy still comes from the original entry"
    );
    assert_eq!(again.label, "tree/");
}
