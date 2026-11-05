// SPDX-License-Identifier: MPL-2.0

//! Command discovery offers what the active view can run and leaves out the
//! rest, while every spelling still resolves and explains itself when typed.

use super::*;

/// The canonical names an empty palette query offers.
fn offered(app: &mut App) -> Vec<&'static str> {
    let typed = std::mem::take(&mut app.command);
    let names = app
        .matching_commands()
        .iter()
        .filter_map(|matched| match matched.spec.id {
            CommandId::Plugin(_) => None,
            id => crate::command::COMMANDS
                .iter()
                .find(|spec| spec.id == id)
                .map(|spec| spec.name),
        })
        .collect();
    app.command = typed;
    names
}

#[track_caller]
fn assert_offers(app: &mut App, names: &[&str]) {
    let offered = offered(app);
    for name in names {
        assert!(offered.contains(name), "{name} should be offered");
    }
}

/// Asserts each command is left out, and that typing it still resolves and
/// says why without running.
#[track_caller]
fn assert_hides(app: &mut App, names: &[&str]) {
    let offered = offered(app);
    let capabilities = app.command_capabilities();
    for name in names {
        assert!(!offered.contains(name), "{name} should not be offered");
        let spec = resolve_command(name).expect("the registry keeps every spelling");
        assert!(
            capabilities.command_availability(spec).reason().is_some(),
            "{name} is hidden, so it has a reason"
        );
    }
}

fn reason(app: &App, name: &str) -> String {
    app.command_capabilities()
        .command_availability(resolve_command(name).unwrap())
        .reason()
        .unwrap_or_default()
        .to_owned()
}

fn project(name: &str) -> PathBuf {
    let root = temporary(name);
    fs::create_dir_all(root.join("child")).unwrap();
    fs::write(root.join("note.txt"), "text\n").unwrap();
    fs::write(root.join("page.md"), "# Title\n\nBody\n").unwrap();
    root.canonicalize().unwrap()
}

fn prepare(app: &mut App) {
    app.prepare_view(FrameGeometry {
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
}

#[test]
fn an_ordinary_file_offers_its_own_actions_and_not_other_views_targets() {
    let root = project("discovery-file");
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(root.join("note.txt")).unwrap();

    assert_offers(
        &mut app,
        &[
            "terminal",
            "terminal-file-directory",
            "explorer",
            "file-picker-directory",
            "fuzzy-grep-directory",
            "diff-disk",
            "diff-this",
            "path",
            "reload",
            "write",
            "write!",
            "write-quit",
            "pipe",
            "zen",
            "help",
            "quit",
            "close",
            "vsplit",
        ],
    );
    assert_hides(
        &mut app,
        &[
            "terminal-directory-root",
            "terminal-selected-directory",
            "open-explorer-system",
            "open-explorer-finder",
            "terminals",
            "terminal-show",
            "terminal-session-directory",
            "terminal-rename",
            "terminal-output",
            "terminal-send",
            "diff-remote",
            "diff-off",
            "render",
            "preview",
            "pipe-cancel",
            "next-git-network-page",
            "previous-git-network-page",
            "git-network-choose-ref",
            "window-close",
            "fullscreen",
            "detach",
            "quit-here",
            "quit-here!",
            "plugin-stop",
            "plugin-restart",
        ],
    );
    assert_eq!(
        reason(&app, "open-explorer-system"),
        "the active view is not a directory buffer"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_typed_hidden_command_explains_itself_without_running_or_redirecting() {
    let root = project("discovery-explicit");
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(root.join("note.txt")).unwrap();
    let buffer = app.active().buffer;

    // The exact spelling offers no row, not even a neighbouring command.
    press(&mut app, ':');
    type_text(&mut app, "terminal-directory-root");
    assert!(app.matching_commands().is_empty());
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "terminal-directory-root");

    // Enter closes the palette, as an attempted command does, so the
    // refusal is echoed where it can be seen rather than behind the prompt.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.command.is_empty());
    assert_eq!(
        app.displayed_status_message(),
        ":terminal-directory-root (Open a terminal in the directory the explorer shows · \
         unavailable: the active view is not a directory buffer)"
    );
    assert!(!app.displayed_status_message_is_error());
    assert!(app.terminals.is_empty());
    assert_eq!(app.active().buffer, buffer);

    // A command with arguments is refused the same way, and an alias
    // resolves to its command's reason.
    type_command(&mut app, "terminal-show 1");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.status.contains("there are no terminal sessions"));
    type_command(&mut app, "do");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.status.contains("no comparison is open"));
    assert_eq!(app.active().buffer, buffer);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_scratch_buffer_saves_to_a_path_but_not_in_place() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert_eq!(app.active_buffer().kind, BufferKind::Scratch);

    assert_offers(&mut app, &["write", "write!", "diff-this", "pipe"]);
    assert_hides(
        &mut app,
        &[
            "write-quit",
            "write-buffer-close",
            "path",
            "reload",
            "diff-disk",
            "terminal-file-directory",
        ],
    );
}

#[test]
fn explorer_rows_decide_where_a_selected_directory_terminal_starts() {
    let root = project("discovery-explorer");
    let mut app = App::new(Config::default(), Some(root.clone())).unwrap();
    assert!(app.active_buffer().is_directory());

    assert_offers(
        &mut app,
        &[
            "terminal-directory-root",
            "open-explorer-system",
            "open-explorer-finder",
            "explorer",
            "file-picker-directory",
            "fuzzy-grep-directory",
            "reload",
            "path",
            "write",
        ],
    );
    assert_hides(
        &mut app,
        &[
            "terminal-file-directory",
            "diff-this",
            "diff-disk",
            "render",
        ],
    );

    let row_of = |app: &App, name: &str| {
        (0..app.active_buffer().len_lines())
            .find(|row| app.active_buffer().line_string(*row).trim_end() == name)
            .unwrap_or_else(|| panic!("{name} is listed"))
    };
    let child = row_of(&app, "child/");
    set_cursor(&mut app, child, 0);
    assert_offers(&mut app, &["terminal-selected-directory"]);
    let file = row_of(&app, "note.txt");
    set_cursor(&mut app, file, 0);
    assert_hides(&mut app, &["terminal-selected-directory"]);
    assert_eq!(
        reason(&app, "terminal-selected-directory"),
        "the selected entry is not a directory"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generated_pages_and_rendered_markdown_offer_only_what_they_support() {
    let root = project("discovery-generated");
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    assert!(app.active_buffer().is_help());
    assert_offers(&mut app, &["help", "quit", "close", "diff-this"]);
    assert_hides(
        &mut app,
        &["write", "write!", "write-quit", "reload", "pipe", "path"],
    );

    app.open_file(root.join("page.md")).unwrap();
    assert_offers(&mut app, &["render", "write", "path"]);
    app.execute_command("render").unwrap();
    assert!(app.active_buffer().markdown_render_source().is_some());
    // The page returns to its source, but is not itself a file to save.
    assert_offers(&mut app, &["render"]);
    assert_hides(&mut app, &["write", "path", "reload"]);

    app.open_file(root.join("note.txt")).unwrap();
    assert_hides(&mut app, &["render"]);
    assert_eq!(reason(&app, "render"), "not a Markdown document");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_commands_follow_sessions_and_the_pane_showing_one() {
    let root = project("discovery-terminals");
    let mut config = Config::default();
    config.editor.auto_close_terminal = false;
    let mut app = App::new(config, None).unwrap();
    app.open_file(root.join("note.txt")).unwrap();
    assert_hides(&mut app, &["terminals", "terminal-show", "terminal-send"]);

    let id = app.terminals.insert_test_session(80, 24);
    assert_offers(
        &mut app,
        &[
            "terminals",
            "terminal-show",
            "terminal-session-directory",
            "terminal-send",
            "terminal-file-directory",
        ],
    );
    assert_hides(&mut app, &["terminal-rename", "terminal-output"]);

    // A pane showing a terminal has no document, whichever file it hides.
    app.show_terminal(id);
    assert_eq!(app.active_terminal(), Some(id));
    assert_offers(
        &mut app,
        &[
            "terminal-rename",
            "terminal-output",
            "terminals",
            "terminal",
        ],
    );
    assert_hides(
        &mut app,
        &[
            "terminal-send",
            "terminal-file-directory",
            "write",
            "path",
            "diff-this",
            "diff-disk",
            "reload",
            "pipe",
        ],
    );
    assert_eq!(
        reason(&app, "write"),
        "the active pane is showing a terminal"
    );

    // An exited session is retained here, so it stays addressable while it
    // can no longer take a paste.
    app.leave_terminal();
    app.apply_terminal_output(TerminalOutput::Exited { id, code: Some(0) });
    assert!(app.terminals.get(id).is_some());
    assert_offers(&mut app, &["terminals", "terminal-show", "terminal-output"]);
    assert_hides(&mut app, &["terminal-send"]);
    assert_eq!(
        reason(&app, "terminal-send"),
        "no running terminal to send to"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_media_and_preview_need_a_frontend_and_a_text_buffer() {
    let root = project("discovery-media");
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(root.join("note.txt")).unwrap();
    assert_hides(&mut app, &["preview"]);
    app.native_media = true;
    assert_offers(&mut app, &["preview"]);

    let buffer = app.active().buffer;
    app.buffers[buffer] = Buffer::virtual_text("[image] picture", "Page 1");
    app.buffers[buffer].media_path = Some(root.join("picture.png"));
    assert_hides(
        &mut app,
        &["preview", "write", "path", "diff-this", "pipe", "reload"],
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pane_commands_follow_the_layout_and_the_maximized_view() {
    let mut app = App::new(Config::default(), None).unwrap();
    prepare(&mut app);
    assert_offers(&mut app, &["zen", "vsplit", "hsplit", "quit"]);
    assert_hides(
        &mut app,
        &[
            "window-close",
            "fullscreen",
            "resize-left",
            "resize-right",
            "resize-top",
            "resize-bottom",
        ],
    );

    app.split_window(Axis::Horizontal).unwrap();
    prepare(&mut app);
    assert_offers(&mut app, &["window-close", "fullscreen"]);
    // The new pane is on the right of the old one, so only its left edge
    // has a neighbour to move.
    assert_offers(&mut app, &["resize-left"]);
    assert_hides(&mut app, &["resize-right", "resize-top", "resize-bottom"]);

    app.execute_command("fullscreen").unwrap();
    assert_offers(&mut app, &["fullscreen"]);
    assert_hides(&mut app, &["window-close"]);
}

#[test]
fn jobs_lifecycle_modes_and_frontends_gate_their_commands() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert_hides(
        &mut app,
        &["pipe-cancel", "git-cancel", "detach", "quit-here"],
    );

    app.pipe.cancellation = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        false,
    )));
    assert_offers(&mut app, &["pipe-cancel"]);
    assert_hides(&mut app, &["pipe"]);
    app.pipe.cancellation = None;
    assert_offers(&mut app, &["pipe"]);

    app.enable_persistent_session();
    app.enable_quit_directory_handoff();
    assert_offers(&mut app, &["detach", "quit-here", "quit-here!"]);
}

#[test]
fn plugin_lifecycle_commands_need_a_configured_plugin_in_a_fitting_phase() {
    use crate::plugin::manager::{Entry, Phase};
    let entry = |phase, valid, enabled| Entry {
        config_index: 0,
        configured_id: "notes".into(),
        enabled,
        valid,
        phase,
        owner: None,
        granted: Vec::new(),
        jobs: 0,
        activities: 0,
        helpers: 0,
        cleanup: 0,
        diagnostic: None,
        compatibility: String::new(),
    };
    let mut app = App::new(Config::default(), None).unwrap();
    assert_offers(&mut app, &["plugins"]);
    assert_hides(&mut app, &["plugin-stop", "plugin-restart"]);

    app.update_plugin_manager(vec![entry(Phase::Starting, true, true)]);
    assert_offers(&mut app, &["plugin-stop"]);
    assert_hides(&mut app, &["plugin-restart"]);

    app.update_plugin_manager(vec![entry(Phase::Running, true, true)]);
    assert_offers(&mut app, &["plugin-stop", "plugin-restart"]);

    app.update_plugin_manager(vec![entry(Phase::Disabled, true, false)]);
    assert_offers(&mut app, &["plugin-stop"]);
    assert_hides(&mut app, &["plugin-restart"]);

    app.update_plugin_manager(vec![entry(Phase::Failed, false, true)]);
    assert_hides(&mut app, &["plugin-stop", "plugin-restart"]);
    // A command that cannot run offers no argument rows either.
    press(&mut app, ':');
    type_text(&mut app, "plugin-stop ");
    assert!(app.matching_plugin_hints().is_none());
}

#[test]
fn git_commands_need_their_file_row_or_view_but_entry_points_stay() {
    use crate::git::{MemoryGitProvider, Repository};
    let root = project("discovery-git");
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(MemoryGitProvider::new(Repository::new(&root)));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(root.join("note.txt")).unwrap();

    assert_offers(
        &mut app,
        &[
            "git-status",
            "git-branches",
            "git-network",
            "git-network-head",
            "git-network-all",
            "git-network-ref",
            "git-blame",
            "git-blame-file",
            "git-diff",
            "git-diff-side-by-side",
            "git-stage",
            "git-unstage",
            "git-discard",
        ],
    );
    assert_hides(
        &mut app,
        &[
            "git-compare",
            "git-merge-branch",
            "git-stash-apply",
            "git-stash-drop",
            "git-stage-hunk",
            "git-unstage-hunk",
            "git-network-choose-ref",
            "next-git-network-page",
            "git-cancel",
        ],
    );

    // A scratch buffer has no file for the file actions to address.
    app.execute_command("buffer-new").unwrap();
    assert_hides(&mut app, &["git-blame", "git-diff", "git-stage"]);
    assert_eq!(
        reason(&app, "git-blame"),
        "this buffer has no file behind it"
    );
    assert_offers(&mut app, &["git-status", "git-network"]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ranking_and_snapshot_rows_match_the_terminal_palette_without_empty_aliases() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, ':');
    type_text(&mut app, "t");
    assert_eq!(
        app.matching_commands().first().map(|row| row.name),
        Some("terminal")
    );

    let matches = app
        .matching_commands()
        .iter()
        .map(|row| (row.spec.name.to_owned(), row.other_names().is_empty()))
        .collect::<Vec<_>>();
    let overlays = app.overlay_snapshots();
    let palette = overlays
        .iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::CommandPalette)
        .unwrap();
    assert_eq!(palette.rows.len(), matches.len());
    for (row, (name, no_aliases)) in palette.rows.iter().zip(&matches) {
        assert!(row.available && !row.dimmed, "{name}");
        assert!(!row.detail.contains("aliases: -"), "{}", row.detail);
        assert!(!row.detail.contains("unavailable"), "{}", row.detail);
        assert_eq!(!row.detail.contains("aliases:"), *no_aliases, "{name}");
    }
    let terminal = palette
        .rows
        .iter()
        .find(|row| row.label.contains(":terminal "))
        .or_else(|| palette.rows.first())
        .unwrap();
    assert!(
        terminal.detail.contains("aliases: t, term"),
        "{}",
        terminal.detail
    );
}

#[test]
fn a_selected_command_keeps_its_row_and_never_completes_another() {
    let root = project("discovery-selection");
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(root.join("note.txt")).unwrap();
    let id = app.terminals.insert_test_session(80, 24);
    press(&mut app, ':');
    type_text(&mut app, "terminal");
    let names = |app: &App| {
        app.matching_commands()
            .iter()
            .map(|row| row.name.to_owned())
            .collect::<Vec<_>>()
    };
    let before = names(&app);
    assert!(before.contains(&"terminal-show".to_owned()));
    let select = |app: &mut App, name: &str| {
        while app
            .selected_command_row(&app.matching_commands())
            .map(|row| app.matching_commands()[row].name.to_owned())
            .as_deref()
            != Some(name)
        {
            key(app, KeyCode::Down, Modifiers::NONE);
        }
    };

    // The last row survives the terminal closing, but moves up.
    let last = before.last().unwrap().clone();
    select(&mut app, &last);
    app.terminals.close(id);
    let after = names(&app);
    assert!(after.len() < before.len());
    let row = app.selected_command_row(&app.matching_commands()).unwrap();
    assert_eq!(after[row], last);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.command.starts_with(&last), "{}", app.command);

    // A selected command that vanishes is not replaced by its successor.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    let id = app.terminals.insert_test_session(80, 24);
    press(&mut app, ':');
    type_text(&mut app, "terminal");
    select(&mut app, "terminal-show");
    app.terminals.close(id);
    assert_eq!(app.selected_command_row(&app.matching_commands()), None);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "terminal");
    assert!(app.status.contains("no longer available"), "{}", app.status);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn editor_mode_leaves_workspace_services_out_of_discovery() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.enter_editor_mode();
    assert_hides(
        &mut app,
        &[
            "terminal",
            "terminals",
            "git-status",
            "plugins",
            "lsp-trust",
        ],
    );
    assert_offers(&mut app, &["help", "open", "write", "quit"]);
    type_command(&mut app, "terminal");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.terminals.is_empty());
    assert!(
        app.status
            .contains(crate::service_health::EDITOR_MODE_REASON),
        "{}",
        app.status
    );
}

#[test]
fn provider_documents_and_commit_messages_offer_their_own_writes() {
    use crate::buffer::{ProviderDocument, ProviderIdentity};
    let mut app = App::new(Config::default(), None).unwrap();
    let provider = Buffer::provider_document(
        ProviderDocument {
            identity: ProviderIdentity {
                configured_plugin: "memory-app".into(),
                provider: "memory".into(),
                key: "notes".into(),
            },
            label: "Remote notes".into(),
            syntax_hint: None,
            version: "v1".into(),
            generation: "g1".into(),
            available: true,
            baseline_epoch: 0,
            uncertain: None,
        },
        "remote text\n".into(),
    );
    app.buffers.push(provider);
    app.syntax.push(None);
    app.active_mut().buffer = app.buffers.len() - 1;
    assert_offers(
        &mut app,
        &[
            "diff-remote",
            "write",
            "write-quit",
            "write-buffer-close",
            "reload",
        ],
    );
    assert_hides(&mut app, &["write!", "diff-disk", "path"]);

    app.buffers.push(Buffer::commit_message(""));
    app.syntax.push(None);
    app.active_mut().buffer = app.buffers.len() - 1;
    assert_offers(&mut app, &["write", "write-quit", "write-buffer-close"]);
    assert_hides(&mut app, &["diff-remote", "reload", "path"]);
}

#[test]
fn a_focused_directory_tree_can_always_be_closed() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert_hides(&mut app, &["window-close"]);
    app.directory_tree.visible = true;
    app.directory_tree.focused = true;
    assert_offers(&mut app, &["window-close"]);
}

#[test]
fn tab_never_turns_a_typed_hidden_command_into_a_neighbour() {
    let root = project("discovery-tab");
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(root.join("note.txt")).unwrap();
    let id = app.terminals.insert_test_session(80, 24);
    app.show_terminal(id);
    app.mode = Mode::Normal;

    // `w` is `write`, which a terminal pane cannot run, while `write-config`
    // and others still match the prefix.
    press(&mut app, ':');
    type_text(&mut app, "w");
    let matches = app.matching_commands();
    assert!(!matches.is_empty());
    assert!(matches.iter().all(|row| row.spec.name != "write"));
    assert_eq!(app.selected_command_row(&matches), None);
    drop(matches);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "w");
    assert!(
        app.status
            .contains("is unavailable: the active pane is showing a terminal"),
        "{}",
        app.status
    );
    let overlays = app.overlay_snapshots();
    let palette = overlays
        .iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::CommandPalette)
        .unwrap();
    assert_eq!(palette.selected, None);

    // Choosing a row deliberately still completes it.
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    let chosen = app.matching_commands()[0].name.to_owned();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.command.starts_with(&chosen), "{}", app.command);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_list_rows_decide_their_row_actions() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };
    let root = project("discovery-git-rows");
    let provider = MemoryGitProvider::new(Repository::new(&root))
        .with_branches(&["feature", "main"], "main")
        .with_status(RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: vec![FileStatus {
                path: PathBuf::from("note.txt"),
                original_path: None,
                index: FileState::Unmodified,
                worktree: FileState::Modified,
            }],
        })
        .with_working("note.txt", "changed\n");
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(provider));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    app.execute_command("git-status").unwrap();
    assert!(app.active_buffer().is_git_status());
    let file_row = (0..app.active_buffer().len_lines())
        .find(|row| app.active_buffer().line_string(*row).contains("note.txt"))
        .unwrap();
    set_cursor(&mut app, file_row, 0);
    assert_offers(
        &mut app,
        &["git-diff", "git-stage", "git-discard", "reload"],
    );
    assert_hides(&mut app, &["git-blame", "git-compare", "git-stash-apply"]);
    let heading = (0..app.active_buffer().len_lines())
        .find(|row| *row != file_row && !app.active_buffer().line_string(*row).trim().is_empty())
        .unwrap();
    set_cursor(&mut app, heading, 0);
    assert_hides(&mut app, &["git-diff", "git-stage"]);
    assert_eq!(reason(&app, "git-diff"), "this row is not a file");
    assert_eq!(reason(&app, "git-stage"), "no files are selected");

    app.execute_command("git-branches").unwrap();
    assert!(app.active_buffer().is_git_branches());
    let branch = (0..app.active_buffer().len_lines())
        .find(|row| app.active_buffer().line_string(*row).contains("feature"))
        .unwrap();
    set_cursor(&mut app, branch, 0);
    assert_offers(&mut app, &["git-compare", "git-merge-branch"]);
    assert_hides(&mut app, &["git-stash-apply", "git-stage"]);

    fs::remove_dir_all(root).unwrap();
}
