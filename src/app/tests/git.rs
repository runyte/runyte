// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::app::git_workflows::RequestedGitViews;
use crate::git::BaseContent;

fn select_remote_branch(app: &mut App, name: &str) {
    let row = app
        .git_state
        .branch_rows()
        .iter()
        .position(|row| {
            row.remote
                .as_ref()
                .is_some_and(|branch| branch.name == name)
        })
        .expect("remote branch row");
    let offset = app.active_buffer().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));
}

#[test]
fn saving_an_external_file_does_not_ask_git_about_it() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("external-file-git-boundary");
    let workspace = root.join("workspace");
    let external = root.join("alacritty.toml");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(&external, "[window]\n").unwrap();
    let root = root.canonicalize().unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let external = external.canonicalize().unwrap();

    // The repository deliberately contains both paths. The workspace is
    // narrower, and that is the boundary editor-owned Git work follows.
    let repository = Repository::new(&root);
    let provider = Rc::new(MemoryGitProvider::new(repository.clone()));
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&workspace, ports).unwrap();

    app.open_file(external.clone()).unwrap();
    assert_eq!(provider.calls(), 0, "opening must not read a staged base");
    assert!(!app.has_visible_git_state());
    assert!(app.git_refresh_spec(&repository).staged_paths.is_empty());
    let buffer = app.active().buffer;
    let end = app.buffers[buffer].len_chars();
    app.buffers[buffer].apply(&Transaction::insert(end, "opacity = 0.9\n"));

    app.save(None, false).unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert_eq!(provider.calls(), 0, "saving must not read a staged base");
    assert_eq!(
        fs::read_to_string(&external).unwrap(),
        "[window]\nopacity = 0.9\n"
    );
    fs::remove_dir_all(root).unwrap();
}

/// The Git gutter and the branch summary come from one tracker, and the
/// tracker is fed by a provider the test owns: no repository is created,
/// and no `git` runs.
#[test]
fn git_marks_reserve_a_gutter_column_and_follow_the_buffer() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-gutter-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "one\ntwo\nthree\n").unwrap();

    let repository = Repository::new(&root);
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(repository).with_staged("source.txt", "one\ntwo\nthree\n"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path.clone()).unwrap();
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };

    // Text that matches the index is marked nowhere, but the column is
    // reserved all the same so editing does not shift the pane sideways.
    let prepared = app.prepare_view(geometry);
    let clean = prepared.pane(0).unwrap().gutter_width;
    assert!(prepared.pane(0).unwrap().changes);
    let snapshot = app.snapshot(&prepared);
    assert!(row_changes(&snapshot).iter().all(Option::is_none));
    assert_eq!(snapshot.status.git_summary.as_deref(), Some("main"));

    let buffer = app.active().buffer;
    app.buffers[buffer].set_text("one\nTWO\nthree\nfour\n");
    let prepared = app.prepare_view(geometry);
    assert_eq!(prepared.pane(0).unwrap().gutter_width, clean);
    let snapshot = app.snapshot(&prepared);

    assert_eq!(
        row_changes(&snapshot)[..4],
        [
            None,
            Some(LineChange::Modified),
            None,
            Some(LineChange::Added),
        ]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_fold_anchor_adds_a_second_gutter_indicator_column() {
    use crate::git::{MemoryGitProvider, Repository};

    let directory = temporary("changed-fold-gutter");
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("source.rs");
    fs::write(&path, "fn changed() {\n    let value = 1;\n}\n").unwrap();

    let repository = Repository::new(&root);
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(repository)
            .with_staged("source.rs", "fn original() {\n    let value = 1;\n}\n"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };

    let ordinary = app.prepare_view(geometry);
    let ordinary_gutter = ordinary.pane(0).unwrap().gutter_width;
    app.fold_all_syntax();
    let folded = app.prepare_view(geometry);
    let folded_pane = folded.pane(0).unwrap();

    assert!(folded_pane.rows[0].folded);
    assert_eq!(folded_pane.gutter_width, ordinary_gutter + 1);
    let snapshot = app.snapshot(&folded);
    let crate::snapshot::SnapshotRow::Text(anchor) = &snapshot.pane(0).unwrap().rows[0] else {
        panic!("fold anchor is visible");
    };
    assert_eq!(anchor.change, Some(LineChange::Modified));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_git_tracking_reuses_the_line_number_text_margin() {
    let directory = temporary("delayed-git-gutter");
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("source.txt");
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers[0].path = Some(path.clone());
    app.buffers[0].kind = BufferKind::File;
    app.buffers[0].set_text("one\ntwo\n");
    app.git.attach(Some(Repository::new(&root)));
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };

    let before = app.prepare_view(geometry);
    assert!(!before.pane(0).unwrap().changes);

    // This is the state transition produced when the asynchronous
    // staged-content request made while opening the file completes.
    app.git
        .apply_staged_content(path, crate::git::BaseContent::Text("one\ntwo\n".to_owned()));
    let after = app.prepare_view(geometry);
    assert!(after.pane(0).unwrap().changes);
    assert_eq!(
        before.pane(0).unwrap().gutter_width,
        after.pane(0).unwrap().gutter_width
    );
    assert_eq!(
        before.pane(0).unwrap().text_width,
        after.pane(0).unwrap().text_width
    );

    fs::remove_dir_all(root).unwrap();
}

/// A file Git has no staged text for keeps the pane exactly as it was.
#[test]
fn an_untracked_file_reserves_no_gutter_column() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = temporary_directory().join(format!(
        "runyte-git-untracked-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("stray.txt");
    fs::write(&path, "new\n").unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(MemoryGitProvider::new(Repository::new(&root))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();

    let prepared = app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });

    assert!(!prepared.pane(0).unwrap().changes);
    assert!(
        app.snapshot(&prepared)
            .status
            .git_summary
            .as_deref()
            .is_some()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_view_preserves_path_selection_and_switches_only_in_persistent_mode() {
    let root = temporary("general-worktree-view");
    let current = root.join("current");
    let linked = root.join("linked");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&linked).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    let note = current.join("note.txt");
    fs::write(&note, "saved\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_file(note).unwrap();
    let file_buffer = app.active().buffer;
    let worktree = |path: PathBuf, branch: &str| Worktree {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some(format!("refs/heads/{branch}")),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    };
    app.open_git_worktrees_result(
        vec![
            worktree(current.clone(), "main"),
            worktree(linked.clone(), "feature"),
        ],
        true,
    );
    assert_eq!(app.key_binding_scope(), BindingScope::GitWorktrees);
    let worktree_buffer = app.active().buffer;
    let linked_offset = app.buffers[worktree_buffer].line_to_offset(1);
    app.active_mut()
        .replace_selection(Selection::point(linked_offset));

    app.buffers[file_buffer].apply(&Transaction::insert(0, "unsaved"));
    app.open_selected_worktree();
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("workspace.mode: persistent"));

    app.enable_persistent_session();
    app.execute(
        crate::command::parse_named_command(
            "session-attach",
            Some(linked.to_string_lossy().as_ref()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(linked.clone()),
        "the command and worktree picker share the switch request"
    );
    app.open_selected_worktree();
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(linked.clone())
    );
    app.open_git_worktrees_result(
        vec![
            worktree(linked.clone(), "feature"),
            worktree(current, "main"),
        ],
        false,
    );
    assert_eq!(
        app.selected_worktree_path().as_deref(),
        Some(linked.as_path())
    );
    app.open_selected_worktree();
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(linked.clone())
    );

    app.open_scratch_buffer();
    app.open_selected_worktree();
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("only available in the worktree list"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_n_in_the_worktree_list_creates_a_new_branch_and_worktree() {
    let root = temporary("general-worktree-new-branch");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_git_worktrees_result(
        vec![Worktree {
            path: root.clone(),
            head: Some("0123456789abcdef".to_owned()),
            branch: Some("refs/heads/main".to_owned()),
            detached: false,
            bare: false,
            locked: None,
            prunable: None,
            missing: false,
            common_dir: root.join(".git"),
        }],
        true,
    );

    context_action(&mut app, 'n');

    assert_eq!(app.prompt_kind, PromptKind::NewWorktreeBranch);
    assert_eq!(app.git_worktree_start.as_deref(), Some("main"));
    assert!(app.git_worktree_new_branch.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_message_picker_fuzzy_matches_bodies_and_keeps_object_identity() {
    let root = temporary("git-commit-search");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let commit = |digit: char, subject: &str, message: &str| {
        let oid = digit.to_string().repeat(40);
        crate::git::CommitSearchEntry {
            summary: CommitSummary {
                abbreviated: oid[..12].to_owned(),
                oid,
                parents: Vec::new(),
                author: "Ada".to_owned(),
                author_time: 1,
                author_date: "2026-08-12".to_owned(),
                subject: subject.to_owned(),
                decorations: Vec::new(),
            },
            message: message.to_owned(),
        }
    };
    app.open_git_commit_search_result(CommitSearchResult {
        commits: vec![
            commit('a', "Unrelated subject", "Unrelated subject\n"),
            commit(
                'b',
                "A visible subject",
                "A visible subject\n\nWorkspace Git refresh behavior.\n",
            ),
        ],
        limited: false,
    });

    for character in "wgr".chars() {
        app.list.as_mut().unwrap().push_filter(character);
    }

    assert_eq!(app.list.as_ref().unwrap().selected_item().unwrap().index, 1);
    let picker = app.list.as_ref().unwrap();
    assert_eq!(
        picker.selected_item().unwrap().label,
        format!("{} A visible subject", "b".repeat(12))
    );
    assert_eq!(
        picker.selected_preview().unwrap(),
        "Ada · 2026-08-12\n\nA visible subject\n\nWorkspace Git refresh behavior.\n"
    );
    let emphasis = picker.selected_preview_emphasis();
    assert!(!emphasis.is_empty());
    assert!(!crate::file_picker::is_direct_match(
        &emphasis,
        &picker.filter
    ));
    assert!(matches!(
        app.selected_list_action(),
        Some(ListAction::GitCommit(oid)) if oid == "b".repeat(40)
    ));

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::ResultList)
        .unwrap();
    assert_eq!(overlay.layout, crate::snapshot::OverlayLayout::Preview);
    assert_eq!(overlay.purpose, crate::snapshot::OverlayPurpose::Picker);
    assert!(matches!(
        overlay.preview,
        Some(crate::snapshot::OverlayPreview::MatchedText { ref lines, ref emphasis })
            if lines[0] == "Ada · 2026-08-12" && lines[2] == "A visible subject"
                && !crate::file_picker::is_direct_match(emphasis, &overlay.query)
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_picker_also_matches_object_ids_authors_and_dates() {
    let commit = |letter: char, author: &str, date: &str, subject: &str| {
        let oid = letter.to_string().repeat(40);
        crate::git::CommitSearchEntry {
            summary: CommitSummary {
                abbreviated: oid[..12].to_owned(),
                oid,
                parents: Vec::new(),
                author: author.to_owned(),
                author_time: 1,
                author_date: date.to_owned(),
                subject: subject.to_owned(),
                decorations: Vec::new(),
            },
            message: format!("{subject}\n"),
        }
    };
    let commits = vec![
        commit('a', "Ada Lovelace", "2026-01-02", "First"),
        commit('b', "Grace Hopper", "2019-11-30", "Second"),
    ];
    // Each query is a field of exactly one commit and not a subsequence of
    // the other's haystack, so a single row surviving proves the match came
    // from that field rather than from the message.
    let full = "b".repeat(40);
    for (query, expected) in [
        ("bbbbbbbbbbbb", 1),
        (full.as_str(), 1),
        ("lovelace", 0),
        ("hopper", 1),
        ("2026-01", 0),
        ("2019-11-30", 1),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.open_git_commit_search_result(CommitSearchResult {
            commits: commits.clone(),
            limited: false,
        });
        for character in query.chars() {
            app.list.as_mut().unwrap().push_filter(character);
        }
        let list = app.list.as_ref().unwrap();
        assert_eq!(list.visible_indices(), vec![expected], "query {query}");
        assert_eq!(
            list.selected_item().unwrap().index,
            expected,
            "query {query}"
        );
    }
}

#[test]
fn filtering_a_git_commit_popup_keeps_the_command_that_opened_it() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.report_completed_action(
        "Space g /",
        "Fuzzy-search commits reachable from HEAD by message, object ID, author, or date",
        CommandOutcome::AsynchronousRequest(None),
    );
    app.open_git_commit_search_result(CommitSearchResult {
        commits: vec![crate::git::CommitSearchEntry {
            summary: CommitSummary {
                abbreviated: "abcdef123456".to_owned(),
                oid: "abcdef1234567890".to_owned(),
                parents: Vec::new(),
                author: "Ada".to_owned(),
                author_time: 1,
                author_date: "2026-08-13".to_owned(),
                subject: "Keep popup feedback".to_owned(),
                decorations: Vec::new(),
            },
            message: "Keep popup feedback\n".to_owned(),
        }],
        limited: false,
    });

    for character in "keep".chars() {
        press(&mut app, character);
    }

    assert_eq!(app.list.as_ref().unwrap().filter, "keep");
    assert_eq!(
        app.displayed_status_message(),
        "Space g / (Fuzzy-search commits reachable from HEAD by message, object ID, author, or date)"
    );

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'l');
    assert_eq!(app.displayed_status_message(), "l (Move right)");
}

#[test]
fn periodic_refresh_defers_to_an_open_prompt_and_to_search_matches() {
    let root = temporary("git-refresh-defer");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let commit = |digit: char, subject: &str| {
        let oid = digit.to_string().repeat(40);
        CommitSummary {
            abbreviated: oid[..7].to_owned(),
            oid,
            parents: Vec::new(),
            author: "Author".to_owned(),
            author_time: 1,
            author_date: "2026-08-12".to_owned(),
            subject: subject.to_owned(),
            decorations: Vec::new(),
        }
    };
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('1', "first"), commit('2', "second")],
            next: None,
            total_pages: 1,
        },
        0,
        true,
    );
    let log = app.active().buffer;
    assert!(app.buffers[log].is_git_log());
    // Step past the idle gate so this exercises the prompt and selection
    // rules on their own.
    app.last_interaction = Instant::now() - Duration::from_secs(3600);

    // A bare caret is where the cursor sits, not a selection to protect.
    let caret = app.buffers[log].line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(caret));
    assert!(!app.interaction_defers_git_refresh());

    // An unfinished command line, which is also how `/`, `s`, and `S`
    // take their query, must survive the timer.
    app.open_prompt(PromptKind::Search(SearchMode::Insensitive));
    assert!(app.interaction_defers_git_refresh());
    app.close_prompt();
    assert!(!app.interaction_defers_git_refresh());

    // The multi-range selection `s` leaves on every match is the visible
    // result of a search and must survive it too.
    let second = app.buffers[log].line_to_offset(2);
    app.active_mut().replace_selection(Selection::new(
        vec![Range::new(caret, caret + 3), Range::new(second, second + 3)],
        0,
    ));
    assert!(app.interaction_defers_git_refresh());

    app.active_mut().replace_selection(Selection::point(caret));
    assert!(!app.interaction_defers_git_refresh());
}

#[test]
fn automatic_refresh_waits_out_a_short_quiet_period_after_the_last_keystroke() {
    let root = temporary("git-refresh-idle");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    // Just acted: a refresh now would move the cursor mid-navigation.
    app.handle_input(InputEvent::Key(KeyStroke::char('j')))
        .unwrap();
    assert!(app.interaction_defers_git_refresh());

    // Still inside the interaction quiet period, independently from the much
    // longer fallback reconciliation interval.
    app.last_interaction = Instant::now() - Duration::from_millis(100);
    assert!(app.interaction_defers_git_refresh());

    // Paused beyond the short quiet period, so reconciliation is welcome.
    app.last_interaction = Instant::now() - Duration::from_secs(1);
    assert!(!app.interaction_defers_git_refresh());

    // Any further input restarts the wait, including pointer input.
    app.handle_input(InputEvent::Key(KeyStroke::char('k')))
        .unwrap();
    assert!(app.interaction_defers_git_refresh());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn refreshing_a_projection_keeps_the_cursor_column() {
    let root = temporary("git-refresh-column");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let buffer = app.open_virtual_diff(
        GeneratedViewIdentity::Named("test-diff".to_owned()),
        "[test]".to_owned(),
        "alpha\nbravo\ncharlie\n",
    );
    assert_eq!(app.active().buffer, buffer);
    let row = 1;
    let column = 3;
    let head = app.buffers[buffer].line_to_offset(row) + column;
    app.active_mut().replace_selection(Selection::point(head));

    // The row keeps its identity, so the cursor keeps its column too.
    app.replace_virtual_preserving_row(buffer, "alpha\nbravo\ndelta\n");
    let after = app.active().head();
    assert_eq!(app.buffers[buffer].offset_to_row(after), row);
    assert_eq!(
        after - app.buffers[buffer].line_to_offset(row),
        column,
        "refresh moved the cursor off its column"
    );

    // A shorter replacement row puts the column at that row's end rather
    // than past it.
    let head = app.buffers[buffer].line_to_offset(2) + 4;
    app.active_mut().replace_selection(Selection::point(head));
    app.replace_virtual_preserving_row(buffer, "alpha\nbravo\nab\n");
    let after = app.active().head();
    let row = app.buffers[buffer].offset_to_row(after);
    assert!(
        after - app.buffers[buffer].line_to_offset(row) <= app.buffers[buffer].line_len(row),
        "cursor landed past the end of its line"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn periodic_refresh_ignores_a_selection_outside_a_git_projection() {
    // A refresh reconciles a tracked file's gutter rather than replacing
    // its text, so selecting in one must not stall the timer forever.
    let root = temporary("git-refresh-source-selection");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("note.txt"), "alpha\nbeta\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_file(root.join("note.txt")).unwrap();
    let source = app.active().buffer;
    assert!(!is_refreshed_projection(&app.buffers[source]));
    app.active_mut()
        .replace_selection(Selection::new(vec![Range::new(0, 5)], 0));
    app.last_interaction = Instant::now() - Duration::from_secs(3600);
    assert!(!app.interaction_defers_git_refresh());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn log_pages_step_forward_and_back_without_taking_a_motion_key() {
    let root = temporary("git-log-paging");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let commit = |digit: char, author_date: &str| {
        let oid = digit.to_string().repeat(40);
        CommitSummary {
            abbreviated: oid[..7].to_owned(),
            oid,
            parents: Vec::new(),
            author: "Author".to_owned(),
            author_time: 1,
            author_date: author_date.to_owned(),
            subject: format!("commit {digit}"),
            decorations: Vec::new(),
        }
    };

    // Page one, with more history behind it.
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![
                commit('1', "2026-08-12"),
                commit('3', "2026-08-14"),
                commit('4', "2026-08-10"),
            ],
            next: Some(LogCursor {
                boundary: "1".repeat(40),
            }),
            total_pages: 2,
        },
        0,
        true,
    );
    let log = app.active().buffer;
    assert_eq!(app.git_state.log_page(), 0);
    assert_eq!(
        app.buffers[log].line_string(0),
        "# page 1/2 | 2026-08-10 - 2026-08-14 |"
    );
    let heading = app.buffers[log].line_string(0);
    let hints = app.buffers[log].row_hints();
    assert_eq!(hints.text(0), Some("(Ctrl-n/p: next/prev page)"));
    let rendered_hint = hints
        .rendered(0, heading.len(), 80 - heading.len())
        .unwrap();
    assert_eq!(
        format!("{heading}{rendered_hint}"),
        "# page 1/2 | 2026-08-10 - 2026-08-14 | (Ctrl-n/p: next/prev page)"
    );
    assert!(heading.len() + rendered_hint.len() <= 80);
    let geometry = FrameGeometry {
        screen: Rect {
            width: 80,
            height: 10,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 8,
            ..Rect::default()
        },
        status: Rect {
            y: 8,
            width: 80,
            height: 1,
            ..Rect::default()
        },
        message: Rect {
            y: 9,
            width: 80,
            height: 1,
            ..Rect::default()
        },
    };
    let prepared = app.prepare_view(geometry);
    let snapshot = app.snapshot(&prepared);
    let heading_row = snapshot.pane(0).unwrap().rows.iter().find_map(|row| {
        let crate::snapshot::SnapshotRow::Text(row) = row else {
            return None;
        };
        (row.document_row == 0).then_some(row)
    });
    let hint_runs = heading_row
        .unwrap()
        .runs
        .iter()
        .filter(|run| run.kind == crate::snapshot::TextRunKind::Hint)
        .map(|run| run.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(hint_runs, [" (Ctrl-n/p: next/prev page)"]);

    // Page two records the cursor that produced it.
    app.open_git_log_result(
        LogRequest {
            cursor: Some(LogCursor {
                boundary: "1".repeat(40),
            }),
            ..LogRequest::default()
        },
        LogPage {
            commits: vec![commit('2', "2025-01-02")],
            next: None,
            total_pages: 2,
        },
        1,
        false,
    );
    assert_eq!(app.git_state.log_page(), 1);
    assert_eq!(
        app.buffers[log].line_string(0),
        "# page 2/2 | 2025-01-02 - 2025-01-02 |"
    );
    assert_eq!(app.git_state.log_rows()[0].oid, "2".repeat(40));
    // The page replaces the view rather than growing it.
    assert_eq!(app.git_state.log_rows().len(), 1);
    // The date is shown, never the raw timestamp it came from.
    assert!(app.buffers[log].line_string(1).contains("2025-01-02"));

    // Going back asks for page one's cursor again, which is none.
    assert_eq!(app.git_state.log_cursors().len(), 2);
    assert_eq!(app.git_state.log_cursors()[0], None);
    assert_eq!(
        app.git_state.log_cursors()[1],
        Some(LogCursor {
            boundary: "1".repeat(40)
        })
    );

    // Paging is bounded at both ends and says so rather than failing.
    app.next_git_log_page();
    assert!(app.status.contains("last page"), "{}", app.status);
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('1', "2026-08-12")],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    app.previous_git_log_page();
    assert!(app.status.contains("first page"), "{}", app.status);
}

#[test]
fn git_log_shows_branch_and_tag_refs_as_a_row_hint_not_text() {
    let root = temporary("git-log-decorations");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let commit = |digit: char, decorations: &[&str]| {
        let oid = digit.to_string().repeat(40);
        CommitSummary {
            abbreviated: oid[..7].to_owned(),
            oid,
            parents: Vec::new(),
            author: "Author".to_owned(),
            author_time: 1,
            author_date: "2026-08-12".to_owned(),
            subject: format!("commit {digit}"),
            decorations: decorations
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    };

    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('1', &["main", "HEAD -> dev"]), commit('2', &[])],
            next: None,
            total_pages: 1,
        },
        0,
        true,
    );
    let log = app.active().buffer;

    // The decorated row's text stays the hash/date/author/subject only;
    // the refs never become part of what a person can select or search.
    let decorated_line = app.buffers[log].line_string(1);
    assert_eq!(decorated_line, "1111111  2026-08-12  Author  commit 1");
    assert!(!decorated_line.contains("main"));
    assert!(!decorated_line.contains("dev"));
    let plain_line = app.buffers[log].line_string(2);
    assert_eq!(plain_line, "2222222  2026-08-12  Author  commit 2");

    let hints = app.buffers[log].row_hints();
    assert_eq!(hints.text(1), Some("(main, HEAD -> dev)"));
    assert_eq!(hints.text(2), None);

    let geometry = FrameGeometry {
        screen: Rect {
            width: 80,
            height: 10,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 8,
            ..Rect::default()
        },
        status: Rect {
            y: 8,
            width: 80,
            height: 1,
            ..Rect::default()
        },
        message: Rect {
            y: 9,
            width: 80,
            height: 1,
            ..Rect::default()
        },
    };
    let prepared = app.prepare_view(geometry);
    let snapshot = app.snapshot(&prepared);
    let decorated_row = snapshot.pane(0).unwrap().rows.iter().find_map(|row| {
        let crate::snapshot::SnapshotRow::Text(row) = row else {
            return None;
        };
        (row.document_row == 1).then_some(row)
    });
    let hint_runs = decorated_row
        .unwrap()
        .runs
        .iter()
        .filter(|run| run.kind == crate::snapshot::TextRunKind::Hint)
        .map(|run| run.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(hint_runs, [" (main, HEAD -> dev)"]);

    // A refresh that drops the ref replaces the hint rather than keeping
    // the stale one around.
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('1', &[]), commit('2', &[])],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    assert_eq!(app.buffers[log].row_hints().text(1), None);
}

#[test]
fn one_very_long_decorated_commit_does_not_hide_hints_on_a_narrow_pane() {
    let root = temporary("git-log-decorations-long-row");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let commit = |digit: char, subject: String, decorations: &[&str]| {
        let oid = digit.to_string().repeat(40);
        CommitSummary {
            abbreviated: oid[..7].to_owned(),
            oid,
            parents: Vec::new(),
            author: "Author".to_owned(),
            author_time: 1,
            author_date: "2026-08-12".to_owned(),
            subject,
            decorations: decorations
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    };

    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![
                commit('1', "x".repeat(2000), &["main"]),
                commit('2', "commit 2".to_owned(), &["dev"]),
            ],
            next: None,
            total_pages: 1,
        },
        0,
        true,
    );
    let log = app.active().buffer;

    // Hints trail their own row rather than sharing one column, so the
    // pathological row cannot push anything else off screen: the heading
    // and the short row's hint both survive it, even on a narrow pane.
    let hints = app.buffers[log].row_hints();
    assert!(hints.text(0).is_some(), "paging reminder was dropped");
    assert_eq!(hints.text(2), Some("(dev)"));

    let geometry = FrameGeometry {
        screen: Rect {
            width: 80,
            height: 10,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 8,
            ..Rect::default()
        },
        status: Rect {
            y: 8,
            width: 80,
            height: 1,
            ..Rect::default()
        },
        message: Rect {
            y: 9,
            width: 80,
            height: 1,
            ..Rect::default()
        },
    };
    let prepared = app.prepare_view(geometry);
    let snapshot = app.snapshot(&prepared);
    let hint_runs_for = |row: usize| {
        let matched = snapshot
            .pane(0)
            .unwrap()
            .rows
            .iter()
            .find_map(|snapshot_row| {
                let crate::snapshot::SnapshotRow::Text(snapshot_row) = snapshot_row else {
                    return None;
                };
                (snapshot_row.document_row == row).then_some(snapshot_row)
            });
        matched
            .unwrap()
            .runs
            .iter()
            .filter(|run| run.kind == crate::snapshot::TextRunKind::Hint)
            .map(|run| run.text.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        !hint_runs_for(0).is_empty(),
        "the paging reminder did not render"
    );
    assert!(
        !hint_runs_for(2).is_empty(),
        "the short commit's ref hint did not render"
    );
}

#[test]
fn the_git_log_leaves_every_motion_key_alone() {
    // The log view may only claim keys that are not motions: `l` used to
    // page, which made it impossible to move right in the view.
    use crate::keymap::Key;

    let keymap = crate::keymap::default_keymap();
    let motions = [
        Key::char('h'),
        Key::char('j'),
        Key::char('k'),
        Key::char('l'),
        Key::char('w'),
        Key::char('b'),
        Key::char('e'),
    ];
    for binding in keymap.bindings() {
        if binding.scope != BindingScope::GitLog {
            continue;
        }
        let keys = binding.sequence.as_slice();
        assert!(
            keys.len() != 1 || !motions.contains(&keys[0]),
            "the Git log bound the motion key {:?} to {:?}",
            keys[0],
            binding.target
        );
    }
}

#[test]
fn log_selection_is_object_stable_and_stale_blame_is_discarded() {
    let root = temporary("git-history-view");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let commit = |digit: char, subject: &str| {
        let oid = digit.to_string().repeat(40);
        CommitSummary {
            abbreviated: oid[..7].to_owned(),
            oid,
            parents: Vec::new(),
            author: "Author".to_owned(),
            author_time: 1,
            author_date: "2026-08-12".to_owned(),
            subject: subject.to_owned(),
            decorations: Vec::new(),
        }
    };
    let first = commit('1', "first");
    let selected = commit('2', "selected");
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![first.clone(), selected.clone()],
            next: Some(LogCursor {
                boundary: selected.oid.clone(),
            }),
            total_pages: 2,
        },
        0,
        true,
    );
    let log = app.active().buffer;
    // The first line is the page heading, so commits start at line 1.
    assert!(app.buffers[log].line_string(0).starts_with("# page 1"));
    assert!(app.buffers[log].line_string(1).contains("2026-08-12"));
    let offset = app.buffers[log].line_to_offset(2);
    app.active_mut().replace_selection(Selection::point(offset));
    assert_eq!(
        app.selected_git_commit_oid().as_deref(),
        Some(selected.oid.as_str())
    );

    // A page that still contains the selected commit keeps the selection
    // on it, wherever the commit moved to.
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('3', "new"), first.clone(), selected.clone()],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    assert_eq!(
        app.selected_git_commit_oid().as_deref(),
        Some(selected.oid.as_str())
    );

    // A page replaces what the view shows, so a commit that is no longer
    // on it falls back to the nearest row rather than being carried over
    // from an earlier page.
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('5', "more than a page of new history")],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    assert_eq!(
        app.selected_git_commit_oid().as_deref(),
        Some("5".repeat(40).as_str())
    );

    // Every pane looking at the log owns its own logical selection, even
    // when another pane is active during refresh.
    let log_pane = app.active_pane;
    app.split(Axis::Horizontal, None).unwrap();
    app.open_scratch_buffer();
    assert_eq!(
        app.git_refresh_spec(&Repository::new(&root)).log_anchors,
        vec!["5".repeat(40)]
    );
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('6', "another prepend"), commit('5', "kept")],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    let selected_line = app.buffers[log].offset_to_row(app.panes[&log_pane].head());
    assert_eq!(
        app.git_state.log_rows()[App::git_log_line_to_row(selected_line).unwrap()].oid,
        "5".repeat(40)
    );
    let repository = Repository::new(&root);
    // A refresh requested for an older selection must not overwrite a
    // newer cursor when its first page has no overlap.
    app.apply_repository_snapshot(
        RepositorySnapshot {
            repository: repository.clone(),
            generation: RepositoryGeneration::from_raw(1),
            started_at: Instant::now(),
            requested: RefreshSpec::default(),
            status: crate::git::RepositoryStatus {
                head: crate::git::Head::Branch("main".to_owned()),
                upstream: None,
                divergence: crate::git::Divergence::default(),
                files: Vec::new(),
            },
            stats: crate::git::StatusStats::default(),
            head_oid: Some("6".repeat(40)),
            staged: Vec::new(),
            branches: None,
            staged_diff: None,
            file_diffs: Vec::new(),
            worktrees: None,
            log: Some(LogPage {
                commits: vec![commit('8', "stale projection")],
                next: None,
                total_pages: 1,
            }),
            requested_log_anchors: vec!["1".repeat(40)],
            reachable_log_anchors: vec!["1".repeat(40)],
            stashes: None,
        },
        true,
        false,
    );
    let stale_line = app.buffers[log].offset_to_row(app.panes[&log_pane].head());
    assert_eq!(
        app.git_state.log_rows()[App::git_log_line_to_row(stale_line).unwrap()].oid,
        "5".repeat(40),
        "a stale snapshot replaced the page it no longer described"
    );
    let rewritten = commit('4', "rewritten history");
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![rewritten.clone()],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    let rewritten_line = app.buffers[log].offset_to_row(app.panes[&log_pane].head());
    assert_eq!(
        app.git_state.log_rows()[App::git_log_line_to_row(rewritten_line).unwrap()].oid,
        rewritten.oid
    );

    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![commit('1', "one"), commit('2', "two"), commit('3', "three")],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    let middle = app.buffers[log].line_to_offset(2);
    app.panes
        .get_mut(&log_pane)
        .unwrap()
        .replace_selection(Selection::point(middle));
    app.open_git_log_result(
        LogRequest::default(),
        LogPage {
            commits: vec![
                commit('7', "seven"),
                commit('8', "eight"),
                commit('9', "nine"),
            ],
            next: None,
            total_pages: 1,
        },
        0,
        false,
    );
    let nearest = app.buffers[log].offset_to_row(app.panes[&log_pane].head());
    assert_eq!(
        nearest, 2,
        "a disappeared row falls back to its nearest row"
    );

    // A periodic result belongs to the view that requested it. Closing
    // that view before completion must not recreate it.
    app.panes.get_mut(&log_pane).unwrap().retarget(0);
    app.closed_buffers.insert(log);
    app.apply_repository_snapshot(
        RepositorySnapshot {
            repository: repository.clone(),
            generation: RepositoryGeneration::from_raw(1),
            started_at: Instant::now(),
            requested: RefreshSpec::default(),
            status: crate::git::RepositoryStatus {
                head: crate::git::Head::Branch("main".to_owned()),
                upstream: None,
                divergence: crate::git::Divergence::default(),
                files: Vec::new(),
            },
            stats: crate::git::StatusStats::default(),
            head_oid: Some("4".repeat(40)),
            staged: Vec::new(),
            branches: None,
            staged_diff: None,
            file_diffs: Vec::new(),
            worktrees: None,
            log: Some(LogPage {
                commits: vec![rewritten],
                next: None,
                total_pages: 1,
            }),
            requested_log_anchors: Vec::new(),
            reachable_log_anchors: Vec::new(),
            stashes: None,
        },
        true,
        false,
    );
    assert!(
        !app.buffers
            .iter()
            .enumerate()
            .any(|(index, buffer)| { !app.closed_buffers.contains(&index) && buffer.is_git_log() })
    );

    // Pagination is also scoped to the requesting live view.
    let late_id = GitRequestId::from_raw(9_999);
    app.git_state.log_requests_mut().insert(
        late_id,
        LogViewRequest {
            buffer: Some(log),
            page: 0,
        },
    );
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: late_id,
        operation: GitOperation::Log {
            repository: repository.clone(),
            request: LogRequest {
                cursor: Some(LogCursor {
                    boundary: "4".repeat(40),
                }),
                limit: 100,
            },
        },
        result: Box::new(Ok(GitResponse::Log {
            request: LogRequest {
                cursor: Some(LogCursor {
                    boundary: "4".repeat(40),
                }),
                limit: 100,
            },
            page: LogPage {
                commits: vec![commit('7', "late page")],
                next: None,
                total_pages: 1,
            },
        })),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(
        !app.buffers
            .iter()
            .enumerate()
            .any(|(index, buffer)| { !app.closed_buffers.contains(&index) && buffer.is_git_log() })
    );

    let source = 0;
    let source_path = root.join("source.txt");
    app.buffers[source].path = Some(source_path.clone());
    app.git.apply_snapshot(
        repository.clone(),
        crate::git::RepositoryStatus {
            head: crate::git::Head::Branch("main".to_owned()),
            upstream: None,
            divergence: crate::git::Divergence::default(),
            files: Vec::new(),
        },
        crate::git::StatusStats::default(),
        Vec::new(),
        false,
    );
    let stale = BlameSource {
        buffer: crate::workspace::BufferId::from_index(source),
        revision: crate::workspace::BufferRevision::from_raw(app.buffers[source].revision()),
        repository: repository.common_dir().to_path_buf(),
        path: source_path.clone(),
        full_file: true,
    };
    app.buffers[source].apply(&Transaction::insert(0, "changed"));
    app.open_git_blame_result(
        stale,
        vec![BlameLine {
            oid: None,
            author: "Not Committed Yet".to_owned(),
            author_time: None,
            author_date: None,
            summary: "live".to_owned(),
            source_line: 1,
            text: "changed".to_owned(),
        }],
    );
    assert!(app.status.contains("discarded stale Git blame"));
    assert!(!app.buffers.iter().any(Buffer::is_git_blame));
    let renamed_path = root.join("renamed.txt");
    let wrong_path = BlameSource {
        buffer: crate::workspace::BufferId::from_index(source),
        revision: crate::workspace::BufferRevision::from_raw(app.buffers[source].revision()),
        repository: repository.common_dir().to_path_buf(),
        path: source_path.clone(),
        full_file: true,
    };
    app.buffers[source]
        .save_as(renamed_path.clone(), false)
        .unwrap();
    app.open_git_blame_result(wrong_path, Vec::new());
    assert!(app.status.contains("discarded stale Git blame"));
    let current = BlameSource {
        buffer: crate::workspace::BufferId::from_index(source),
        revision: crate::workspace::BufferRevision::from_raw(app.buffers[source].revision()),
        repository: repository.common_dir().to_path_buf(),
        path: renamed_path,
        full_file: true,
    };
    app.open_git_blame_result(
        current,
        vec![BlameLine {
            oid: None,
            author: "Not Committed Yet".to_owned(),
            author_time: None,
            author_date: None,
            summary: "live".to_owned(),
            source_line: 1,
            text: "changed".to_owned(),
        }],
    );
    assert!(app.active_buffer().is_git_blame());
    assert_eq!(app.key_binding_scope(), BindingScope::GitBlame);
    app.open_selected_git_commit();
    assert!(app.status.contains("uncommitted"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_mutations_feed_the_generic_long_running_action_snapshot() {
    let root = temporary("git-long-running-action");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.apply_git_service_event(GitServiceEvent::Progress(GitServiceProgress {
        id: GitRequestId::from_raw(42),
        operation: "push",
        repository: root.clone(),
        state: GitServiceState::Running,
        started_at: Some(Instant::now() - Duration::from_millis(640)),
        cancellable: true,
        mutation: true,
    }));

    let prepared = app.prepare_view(git_test_geometry());
    let action = app
        .snapshot(&prepared)
        .status
        .long_running_action
        .expect("running mutation should own the status row");
    assert_eq!(action.label, "Git · running push");
    assert_eq!(action.detail, root.display().to_string());
    assert!(action.elapsed_millis >= 640);
    assert_eq!(action.cancel_hint.as_deref(), Some(":git-cancel"));

    app.git_state
        .progress_mut()
        .get_mut(&GitRequestId::from_raw(42))
        .unwrap()
        .mutation = false;
    assert!(!app.has_long_running_action());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn blame_refuses_oversized_and_binary_buffers_before_service_submission() {
    let root = temporary("git-blame-preflight");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.buffers[0].path = Some(path.clone());
    app.git.apply_snapshot(
        Repository::new(&root),
        crate::git::RepositoryStatus {
            head: crate::git::Head::Branch("main".to_owned()),
            upstream: None,
            divergence: crate::git::Divergence::default(),
            files: Vec::new(),
        },
        crate::git::StatusStats::default(),
        Vec::new(),
        false,
    );
    app.buffers[0]
        .discard_changes_to(&"x".repeat(MAX_BLAME_INPUT_BYTES + 1))
        .unwrap();
    app.request_git_blame(false);
    assert!(app.status.contains("accepts buffers up to"));
    assert!(app.git_state.progress().is_empty());

    app.buffers[0].discard_changes_to("text\0binary").unwrap();
    app.request_git_blame(false);
    assert!(app.status.contains("binary buffers cannot be blamed"));
    assert!(app.git_state.progress().is_empty());
    assert!(!app.buffers.iter().any(Buffer::is_git_blame));
    fs::remove_dir_all(root).unwrap();
}

/// Marks measured against a base that has moved are wrong until asked to
/// re-read it, which is the whole reason the command exists.
#[test]
fn git_refresh_rereads_the_base_after_it_changes_outside_the_editor() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = temporary_directory().join(format!(
        "runyte-git-refresh-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "after\n").unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut provider = MemoryGitProvider::new(Repository::new(&root));
    provider.set_staged("source.txt", "before\n");
    ports.replace_git(Box::new(provider));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let prepared = app.prepare_view(geometry);
    assert_eq!(
        row_changes(&app.snapshot(&prepared))[0],
        Some(LineChange::Modified)
    );

    app.execute_command("git-refresh").unwrap();

    assert!(app.status.starts_with("git: main"));
    assert!(!app.status_error);

    fs::remove_dir_all(root).unwrap();
}

/// The change of every visible row, in screen order.
fn row_changes(snapshot: &crate::snapshot::EditorSnapshot) -> Vec<Option<LineChange>> {
    snapshot
        .pane(0)
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row {
            crate::snapshot::SnapshotRow::Text(row) => Some(row.change),
            crate::snapshot::SnapshotRow::Placeholder
            | crate::snapshot::SnapshotRow::Padding
            | crate::snapshot::SnapshotRow::Filler => None,
        })
        .collect()
}

/// Staging is the payoff of measuring against the index: the marks for the
/// lines you staged are the ones that go away.
#[test]
fn staging_the_active_file_clears_its_marks_and_unstaging_brings_them_back() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-stage-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "one\nTWO\n").unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_staged("source.txt", "one\ntwo\n")
            .with_working("source.txt", "one\nTWO\n"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();
    let geometry = git_test_geometry();
    let prepared = app.prepare_view(geometry);
    assert_eq!(
        row_changes(&app.snapshot(&prepared))[1],
        Some(LineChange::Modified)
    );

    app.execute_command("git-stage").unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.status, "staged source.txt");
    let prepared = app.prepare_view(geometry);
    assert!(
        row_changes(&app.snapshot(&prepared))
            .iter()
            .all(Option::is_none),
        "staged lines still carry marks"
    );

    app.execute_command("git-unstage").unwrap();

    assert_eq!(app.status, "unstaged source.txt");
    let prepared = app.prepare_view(geometry);
    assert!(
        !prepared.pane(0).unwrap().changes,
        "an unstaged never-committed file has no base to compare against"
    );

    fs::remove_dir_all(root).unwrap();
}

/// Staging records the file on disk, so a buffer that has not been written
/// has to say which text was recorded rather than let the reader assume.
#[test]
fn staging_an_unsaved_buffer_says_which_text_it_recorded() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-dirty-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "one\n").unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_staged("source.txt", "one\n")
            .with_working("source.txt", "one\n"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();
    let buffer = app.active().buffer;
    // A real edit rather than `set_text`, which replaces content
    // authoritatively and leaves the buffer clean.
    app.buffers[buffer].apply(&Transaction::insert(0, "edited "));
    assert!(app.buffers[buffer].dirty);

    app.execute_command("git-stage").unwrap();

    assert!(app.status.contains("as written on disk"), "{}", app.status);
    assert!(app.status.contains("unsaved changes"), "{}", app.status);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_diff_and_index_views_are_read_only_buffers_that_name_what_they_show() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-views-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "one\n").unwrap();

    let patch = "@@ -1 +1 @@\n-one\n+two\n";
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_staged("source.txt", "one\n")
            .with_diff(patch)
            .with_status(crate::git::RepositoryStatus {
                head: crate::git::Head::Branch("main".to_owned()),
                upstream: None,
                divergence: crate::git::Divergence::default(),
                files: vec![crate::git::FileStatus {
                    path: PathBuf::from("source.txt"),
                    original_path: None,
                    index: crate::git::FileState::Modified,
                    worktree: crate::git::FileState::Unmodified,
                }],
            }),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();

    app.execute_command("git-diff").unwrap();
    let diff_buffer = app.active().buffer;
    let diff = app.buffers[diff_buffer].to_string();
    assert_eq!(
        app.buffers[diff_buffer].display_name(),
        "[git diff source.txt]"
    );
    assert!(app.buffers[diff_buffer].is_read_only());
    assert!(diff.starts_with("# not staged · source.txt"), "{diff}");
    assert!(diff.contains(patch), "{diff}");

    assert!(
        app.buffers[diff_buffer].is_diff(),
        "the diff view must be readable as a patch"
    );

    app.execute_command("git-index").unwrap();
    let index_buffer = app.active().buffer;
    let index = app.buffers[index_buffer].to_string();
    assert_eq!(app.buffers[index_buffer].display_name(), "[git index]");
    assert!(index.starts_with("# staged for commit · 1 file"), "{index}");
    assert!(index.contains("  M source.txt"), "{index}");
    assert!(index.contains(patch), "{index}");
    assert!(app.buffers[index_buffer].is_diff());

    // The classification reaches the snapshot, which is what a frontend
    // colours from.
    let prepared = app.prepare_view(git_test_geometry());
    let rows = app
        .snapshot(&prepared)
        .pane(0)
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row {
            crate::snapshot::SnapshotRow::Text(row) => Some(row.diff),
            crate::snapshot::SnapshotRow::Placeholder
            | crate::snapshot::SnapshotRow::Padding
            | crate::snapshot::SnapshotRow::Filler => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rows[0], Some(crate::git::DiffLine::Meta), "the heading");
    assert!(
        rows.contains(&Some(crate::git::DiffLine::Added)),
        "the patch's added line: {rows:?}"
    );
    assert!(
        rows.contains(&Some(crate::git::DiffLine::Removed)),
        "the patch's removed line: {rows:?}"
    );
    assert!(
        rows.contains(&Some(crate::git::DiffLine::Hunk)),
        "the hunk position: {rows:?}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn space_g_shift_d_compares_complete_index_and_worktree_versions() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-side-by-side");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "working\n").unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_staged("source.txt", "staged\n")
            .with_working("source.txt", "working\n"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();

    for character in [' ', 'g', 'D'] {
        press(&mut app, character);
    }

    let right_pane = app.active_pane;
    let session = app.diff_session(right_pane).unwrap();
    let left = session.side(Side::Left);
    let right = session.side(Side::Right);
    assert_eq!(
        app.buffers[left.buffer].display_name(),
        "[index source.txt]"
    );
    assert_eq!(app.buffers[left.buffer].to_string(), "staged\n");
    assert_eq!(
        app.buffers[right.buffer].display_name(),
        "[worktree source.txt]"
    );
    assert_eq!(app.buffers[right.buffer].to_string(), "working\n");
    assert!(app.buffers[left.buffer].is_read_only());
    assert!(app.buffers[right.buffer].is_read_only());
    assert_eq!(
        session.change(Side::Left, 0),
        Some(crate::diff::Change::Changed)
    );
    assert_eq!(
        session.change(Side::Right, 0),
        Some(crate::diff::Change::Changed)
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn side_by_side_git_views_make_added_and_removed_sides_empty() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let cases = [
        (
            "added",
            FileStatus {
                path: PathBuf::from("source.txt"),
                original_path: None,
                index: FileState::Unmodified,
                worktree: FileState::Untracked,
            },
            None,
            Some("new\n"),
        ),
        (
            "removed",
            FileStatus {
                path: PathBuf::from("source.txt"),
                original_path: None,
                index: FileState::Unmodified,
                worktree: FileState::Deleted,
            },
            Some("old\n"),
            None,
        ),
    ];
    for (name, file, staged, working) in cases {
        let root = temporary(name);
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut provider =
            MemoryGitProvider::new(Repository::new(&root)).with_status(RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: vec![file],
            });
        if let Some(text) = staged {
            provider = provider.with_staged("source.txt", text);
        }
        if let Some(text) = working {
            provider = provider.with_working("source.txt", text);
        }
        let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        )))));
        ports.replace_git(Box::new(provider));
        let mut app = App::new_in_isolated_project(&root, ports).unwrap();
        app.execute_command("git-status").unwrap();

        for character in [' ', 'g', 'D'] {
            press(&mut app, character);
        }

        let session = app.diff_session(app.active_pane).unwrap();
        let left = session.side(Side::Left);
        let right = session.side(Side::Right);
        assert_eq!(
            app.buffers[left.buffer].to_string(),
            staged.unwrap_or_default()
        );
        assert_eq!(
            app.buffers[right.buffer].to_string(),
            working.unwrap_or_default()
        );
        assert_eq!(
            session.alignment().lines(Side::Left),
            usize::from(staged.is_some())
        );
        assert_eq!(
            session.alignment().lines(Side::Right),
            usize::from(working.is_some())
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn space_g_b_opens_local_branches_and_enter_checks_one_out() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branches");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    for character in [' ', 'g', 'b'] {
        press(&mut app, character);
    }

    assert!(app.active_buffer().is_git_branches());
    assert!(app.active_buffer().is_read_only());
    assert_eq!(app.key_binding_scope(), BindingScope::GitBranches);
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n  feature\n* main\n\nRemote\n  no remote branches known"
    );
    // Opening follows the current branch. Move to the branch above it and
    // activate the row through the branch-list binding.
    press(&mut app, 'k');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(provider.checkouts(), vec!["feature"]);
    assert_eq!(app.status, "checked out feature");
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n* feature\n  main\n\nRemote\n  no remote branches known"
    );
    assert_eq!(app.cursor_position(), Position::new(1, 0));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_untracked_remote_row_creates_and_checks_out_a_tracking_local_branch() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-remote-branch-checkout");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["main"], "main")
            .with_remote_branches(&["origin/topic"]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    select_remote_branch(&mut app, "origin/topic");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        provider.creations(),
        vec![("topic".to_owned(), "refs/remotes/origin/topic".to_owned())]
    );
    assert_eq!(provider.checkouts(), vec!["topic"]);
    assert_eq!(app.status, "created topic tracking origin/topic");
    assert!(
        app.active_buffer()
            .to_string()
            .contains("origin/topic [tracked by: topic]")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_remote_row_checks_out_its_existing_local_tracking_branch() {
    use crate::git::{MemoryGitProvider, Repository, Upstream};

    let root = temporary("git-remote-existing-tracker");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            .with_branch_detail(
                "feature",
                Some(Upstream::origin("topic", Some(Default::default()))),
                false,
            )
            .with_remote_branches(&["origin/topic"]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    select_remote_branch(&mut app, "origin/topic");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(provider.checkouts(), vec!["feature"]);
    assert_eq!(app.status, "checked out feature");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_remote_row_with_several_trackers_asks_which_local_branch_to_use() {
    use crate::git::{MemoryGitProvider, Repository, Upstream};

    let root = temporary("git-remote-several-trackers");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let upstream = Some(Upstream::origin("topic", Some(Default::default())));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main", "release"], "main")
            .with_branch_detail("feature", upstream.clone(), false)
            .with_branch_detail("release", upstream, false)
            .with_remote_branches(&["origin/topic"]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    select_remote_branch(&mut app, "origin/topic");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        app.list.as_ref().map(|list| list.title.as_str()),
        Some("Local branches tracking origin/topic")
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(provider.checkouts(), vec!["feature"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_conflicting_default_remote_branch_name_opens_an_editable_name_prompt() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-remote-name-conflict");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["main", "topic"], "main")
            .with_remote_branches(&["origin/topic"]),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    select_remote_branch(&mut app, "origin/topic");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.prompt_kind, PromptKind::NewBranch);
    assert_eq!(app.command, "topic");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_slash_remote_uses_its_actual_branch_name_for_checkout_and_worktrees() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-slash-remote-name");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["main"], "main")
            .with_remote_branch("fork/team", "main"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    select_remote_branch(&mut app, "fork/team/main");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.prompt_kind, PromptKind::NewBranch);
    assert_eq!(app.command, "main");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    context_action(&mut app, 'w');

    assert_eq!(app.prompt_kind, PromptKind::NewWorktreeBranch);
    assert_eq!(app.command, "main");
    assert_eq!(
        app.git_worktree_start.as_deref(),
        Some("refs/remotes/fork/team/main")
    );
    assert_eq!(
        app.git_worktree_upstream.as_deref(),
        Some("refs/remotes/fork/team/main")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_w_starts_a_worktree_from_the_selected_local_branch() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-worktree");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    context_action(&mut app, 'w');

    assert_eq!(app.prompt_kind, PromptKind::WorktreeDestination);
    assert_eq!(app.git_worktree_start.as_deref(), Some("feature"));
    assert!(app.git_worktree_new_branch.is_none());
    assert!(app.git_worktree_upstream.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_w_on_an_untracked_remote_prepares_a_tracking_branch_in_the_worktree() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-remote-worktree");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["main"], "main")
            .with_remote_branches(&["origin/topic"]),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    select_remote_branch(&mut app, "origin/topic");

    context_action(&mut app, 'w');

    assert_eq!(app.prompt_kind, PromptKind::WorktreeDestination);
    assert_eq!(
        app.git_worktree_start.as_deref(),
        Some("refs/remotes/origin/topic")
    );
    assert_eq!(app.git_worktree_new_branch.as_deref(), Some("topic"));
    assert_eq!(
        app.git_worktree_upstream.as_deref(),
        Some("refs/remotes/origin/topic")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_w_refuses_a_branch_that_already_has_a_checkout() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-existing-worktree");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let checkout = root.parent().unwrap().join("feature-checkout");
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            .with_branch_checkout("feature", checkout.clone()),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    context_action(&mut app, 'w');

    assert!(app.status_error);
    assert!(app.status.contains("already checked out"), "{}", app.status);
    assert!(
        app.status.contains(&checkout.display().to_string()),
        "{}",
        app.status
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_hidden_live_terminal_requires_exact_branch_name_before_checkout() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-terminal-confirmation");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.leave_terminal();
    assert!(app.terminals.get(terminal).unwrap().live());
    assert_eq!(app.active_terminal(), None);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(provider.checkouts().is_empty());
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Switch branch");
    assert_eq!(overlay.actions[0].label, "confirm exact text");
    assert_eq!(overlay.input, crate::snapshot::OverlayInput::Text);
    let message = overlay.message.unwrap();
    assert!(message.contains("Switch to branch feature."), "{message}");
    assert!(
        message.contains("terminal session is still running"),
        "{message}"
    );
    assert!(message.contains("Type feature exactly"), "{message}");

    let transported: InputEvent = crate::protocol::InputEvent::Text("main".to_owned()).into();
    app.handle_input(transported).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.git_branch_switch.is_some());
    assert!(provider.checkouts().is_empty());
    assert_eq!(
        app.status,
        "type the exact branch name before switching branches"
    );

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.git_branch_switch.is_none());
    assert_eq!(app.status, "checkout cancelled; the branch was not changed");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.git_branch_switch.is_some());
    assert!(provider.checkouts().is_empty());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let transported: InputEvent = crate::protocol::InputEvent::Text("feature".to_owned()).into();
    app.handle_input(transported).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(provider.checkouts(), vec!["feature"]);
    assert_eq!(app.status, "checked out feature");
    assert!(app.git_branch_switch.is_none());

    app.apply_terminal_output(crate::terminal::TerminalOutput::Exited {
        id: terminal,
        code: Some(0),
    });
    assert!(app.terminals.get(terminal).is_none());
    press(&mut app, 'j');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(provider.checkouts(), vec!["feature", "main"]);
    assert_eq!(app.status, "checked out main");
    assert!(app.git_branch_switch.is_none());

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn confirmed_terminal_branch_checkout_is_submitted_to_the_git_service() {
    use crate::git::{GitOperation, GitServiceHandle, MemoryGitProvider, Repository};

    let root = temporary("git-branch-terminal-service");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.leave_terminal();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    let transported: InputEvent = crate::protocol::InputEvent::Text("feature".to_owned()).into();
    app.handle_input(transported).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(matches!(
        operations
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        GitOperation::Discover { .. }
    ));
    assert!(matches!(
        operations
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        GitOperation::Mutate {
            mutation: GitMutation::Checkout { branch },
            ..
        } if branch == "feature"
    ));
    assert!(app.git_branch_switch.is_none());

    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

/// `Tab n` starts a branch at the selected row and switches to it, so the
/// list that comes back marks the new branch rather than the old one.
#[test]
fn n_creates_a_branch_at_the_selected_row_and_switches_to_it() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-new");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();

    // A character-taking command owns Tab before this view's menu can.
    press(&mut app, 'r');
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.context_action_menu.is_none());
    assert!(app.awaiting_character_command().is_none());
    assert_eq!(app.status, "expected a character");

    // The caret opens on `main`; start the new branch from `feature`.
    press(&mut app, 'k');

    // The bare key keeps its global search-next meaning in this buffer.
    press(&mut app, 'n');
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.prompt_kind, PromptKind::Command);
    assert!(provider.creations().is_empty());

    context_action(&mut app, 'n');

    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.prompt_kind, PromptKind::NewBranch);
    for character in "spike".chars() {
        press(&mut app, character);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        provider.creations(),
        vec![("spike".to_owned(), "feature".to_owned())]
    );
    assert_eq!(app.status, "created spike from feature");
    assert!(!app.status_error);
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n  feature\n  main\n* spike\n\nRemote\n  no remote branches known"
    );
    assert_eq!(app.cursor_position(), Position::new(3, 0));

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn creating_a_branch_with_a_live_terminal_requires_exact_name_confirmation() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-create-terminal-confirmation");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.leave_terminal();

    context_action(&mut app, 'n');
    for character in "spike".chars() {
        press(&mut app, character);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(provider.creations().is_empty());
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Switch branch");
    let message = overlay.message.unwrap();
    assert!(
        message.contains("Create and switch to branch spike."),
        "{message}"
    );
    assert!(message.contains("Type spike exactly"), "{message}");

    let transported: InputEvent = crate::protocol::InputEvent::Text("spike".to_owned()).into();
    app.handle_input(transported).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        provider.creations(),
        vec![("spike".to_owned(), "feature".to_owned())]
    );
    assert_eq!(app.status, "created spike from feature");
    assert!(app.git_branch_switch.is_none());

    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

/// An abandoned prompt creates nothing, and leaves nothing behind for the
/// next prompt to inherit.
#[test]
fn an_abandoned_new_branch_prompt_creates_nothing() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-new-escape");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();

    context_action(&mut app, 'n');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    assert!(provider.creations().is_empty());
    assert!(app.git_branch_start.is_none());
    assert_eq!(app.mode, Mode::Normal);

    // A name of nothing but spaces is refused rather than handed to Git.
    context_action(&mut app, 'n');
    press(&mut app, ' ');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(provider.creations().is_empty());
    assert!(app.status_error);
    assert_eq!(app.status, "a new branch needs a name");

    fs::remove_dir_all(root).unwrap();
}

/// `D` asks first, says what deleting an unmerged branch costs, and leaves
/// the caret in the list rather than sending it back to the top.
#[test]
fn shift_d_deletes_a_branch_only_after_a_confirmation() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-delete");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main", "spike"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();

    // The branch this working tree is on has no delete at all.
    context_action(&mut app, 'D');
    assert!(app.status_error);
    assert_eq!(
        app.status,
        "cannot delete the branch this working tree is on; check out another branch first"
    );
    assert!(app.git_branch_deletion.is_none());

    // Escape on another branch keeps it.
    press(&mut app, 'k');
    context_action(&mut app, 'D');
    assert!(app.status.starts_with("Delete branch feature"));
    assert!(
        app.status.contains("type feature exactly"),
        "{}",
        app.status
    );
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Delete branch");
    assert_eq!(overlay.actions[0].label, "confirm exact text");
    assert_eq!(overlay.input, crate::snapshot::OverlayInput::Text);
    assert_eq!(overlay.query, "");
    let message = overlay.message.unwrap();
    assert!(message.contains("feature"), "{message}");
    assert!(message.contains("type feature exactly"), "{message}");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.git_branch_deletion.is_some());
    assert!(provider.deletions().is_empty());
    assert!(app.status_error);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(provider.deletions().is_empty());
    assert_eq!(app.status, "delete cancelled; the branch is still there");

    context_action(&mut app, 'D');
    let transported: InputEvent = crate::protocol::InputEvent::Text("feature".to_owned()).into();
    app.handle_input(transported).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(provider.deletions(), vec![("feature".to_owned(), true)]);
    assert_eq!(app.status, "deleted feature");
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n* main\n  spike\n\nRemote\n  no remote branches known"
    );
    // The row the deleted branch occupied is now `main`, and the caret
    // stayed on it rather than jumping.
    assert_eq!(app.cursor_position(), Position::new(1, 0));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_async_branch_deletion_preflights_never_open_a_confirmation() {
    use crate::{
        app::git_workflows::DeletionPreflight,
        git::{MemoryGitProvider, Repository},
    };

    let root = temporary("git-branch-delete-stale-preflight");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = Repository::new(&root);
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(repository.clone()).with_branches(&["feature", "main"], "main"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');
    let source_buffer = app.active().buffer;
    let old_id = GitRequestId::from_raw(41);
    let latest_id = GitRequestId::from_raw(42);
    app.git_state.branch_deletion_request = Some(DeletionPreflight {
        id: latest_id,
        source_buffer,
        interaction_generation: app.next_action_id,
        target: "feature".to_owned(),
    });
    let operation = || GitOperation::PrepareBranchDeletion {
        cascade_checkout: None,
        repository: repository.clone(),
        branch: "feature".to_owned(),
    };
    let response = || {
        Box::new(Ok(GitResponse::PreparedBranchDeletion(
            BranchDeletionPlan {
                branch: "feature".to_owned(),
                tip: "1".repeat(40),
                upstream: None,
                retaining_branches: vec!["main".to_owned()],
                required_authorization: DeletionAuthorization::Enter,
            },
        )))
    };

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: old_id,
        operation: operation(),
        result: response(),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(app.git_branch_deletion.is_none());
    assert_eq!(
        app.git_state
            .branch_deletion_request
            .as_ref()
            .map(|pending| pending.id),
        Some(latest_id)
    );

    app.open_file(root.join("elsewhere.txt")).unwrap();
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: latest_id,
        operation: operation(),
        result: response(),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(app.git_branch_deletion.is_none());
    assert!(app.git_state.branch_deletion_request.is_none());
    fs::remove_dir_all(root).unwrap();
}

/// A branch retained by another local branch needs only the ordinary
/// confirmation, even though the guarded mutation uses `-D` after revalidation.
#[test]
fn deleting_a_merged_branch_is_not_forced() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-delete-merged");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            .with_branch_detail("feature", None, true),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    context_action(&mut app, 'D');
    assert_eq!(
        app.status,
        "Delete branch feature.\nIts commits are retained by local branch main.\nPress Enter to continue.\nEscape keeps it."
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(provider.deletions(), vec![("feature".to_owned(), true)]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn branch_deletion_cascades_through_its_checkout_and_asks_for_the_branch_name() {
    use crate::git::{MemoryGitProvider, Repository, Worktree};

    let root = temporary("git-branch-delete-worktree");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let linked = root.join("linked");
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            // Retained by `main`, so the branch alone would have settled for
            // Enter. The cascade below it is what raises the bar.
            .with_branch_detail("feature", None, true)
            .with_branch_checkout("feature", linked.clone())
            .with_worktrees(vec![
                Worktree {
                    path: root.clone(),
                    head: Some("1".repeat(40)),
                    branch: Some("refs/heads/main".to_owned()),
                    detached: false,
                    bare: false,
                    locked: None,
                    prunable: None,
                    missing: false,
                    common_dir: root.join(".git"),
                },
                Worktree {
                    path: linked.clone(),
                    head: Some("1".repeat(40)),
                    branch: Some("refs/heads/feature".to_owned()),
                    detached: false,
                    bare: false,
                    locked: None,
                    prunable: None,
                    missing: false,
                    common_dir: root.join(".git"),
                },
            ]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    context_action(&mut app, 'D');

    // Every level is named before anything is accepted, rather than being
    // discovered one refusal at a time.
    assert!(!app.status_error, "{}", app.status);
    assert!(
        app.status.contains("Delete branch feature."),
        "{}",
        app.status
    );
    assert!(app.status.contains("This also:"), "{}", app.status);
    assert!(
        app.status
            .contains(&format!("· removes worktree {}", linked.display())),
        "{}",
        app.status
    );
    assert!(
        app.status.contains("Type feature exactly to continue."),
        "{}",
        app.status
    );

    // A compound action asks for typed text even where the branch tip alone
    // would have accepted Enter.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert!(provider.deletions().is_empty());
    assert!(provider.removed_worktrees().is_empty());
    assert!(
        app.git_branch_deletion.is_some(),
        "the review is still open"
    );

    for character in "feature".chars() {
        press(&mut app, character);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    // Bottom up: the checkout goes before the branch that is checked out in it.
    assert_eq!(provider.removed_worktrees(), vec![linked.clone()]);
    assert_eq!(provider.deletions(), vec![("feature".to_owned(), true)]);
    assert!(
        app.status.contains("deleted branch feature")
            && app
                .status
                .contains(&format!("removed worktree {}", linked.display())),
        "{}",
        app.status
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn branch_deletion_refuses_more_checkouts_than_a_cascade_can_answer_for() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-branch-delete-many-worktrees");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let first = root.join("first");
    let second = root.join("second");
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            .with_branch_checkout("feature", first.clone())
            .with_branch_checkout("feature", second.clone()),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    context_action(&mut app, 'D');

    assert!(app.status_error);
    assert!(app.status.contains("checked out"), "{}", app.status);
    assert!(app.status.contains(":git-worktrees"), "{}", app.status);
    assert!(app.status.contains(&first.to_string_lossy().to_string()));
    assert!(app.git_branch_deletion.is_none());
    assert!(provider.deletions().is_empty());

    fs::remove_dir_all(root).unwrap();
}

/// `p` fast-forwards the current branch and `P` publishes the row's, in the
/// branch list and in the changed-file list alike.
#[test]
fn p_pulls_the_current_branch_and_shift_p_pushes_the_selected_one() {
    use crate::git::{Divergence, MemoryGitProvider, Repository, Upstream};

    let root = temporary("git-network");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            // Ahead only, so the pull below has a fast-forward to make. A
            // branch that had drifted both ways would be offered a replay
            // instead, which
            // `pulling_a_diverged_branch_offers_to_replay_the_local_commits`
            // covers.
            .with_branch_detail(
                "main",
                Some(Upstream::origin(
                    "main",
                    Some(Divergence {
                        ahead: 2,
                        behind: 0,
                    }),
                )),
                true,
            ),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n  feature\n* main [↑2]\n\nRemote\n  no remote branches known"
    );

    // The caret opens on the current branch, so `p` acts on it.
    context_action(&mut app, 'p');

    assert_eq!(provider.pulls(), 1);
    assert!(!app.status_error, "{}", app.status);
    // Fast-forwarded, so nothing is left to come down.
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n  feature\n* main [↑2]\n\nRemote\n  no remote branches known"
    );

    context_action(&mut app, 'P');

    assert_eq!(provider.pushes(), vec!["main"]);
    assert_eq!(app.status, "pushed main");
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n  feature\n* main [=]\n\nRemote\n  no remote branches known"
    );

    // `P` on another row publishes that row rather than the current branch:
    // pushing a branch touches no working tree, so it is not restricted.
    press(&mut app, 'k');
    context_action(&mut app, 'P');

    assert_eq!(provider.pushes(), vec!["main", "feature"]);
    assert_eq!(app.status, "pushed feature");

    // `p` on another row is refused, because a pull merges into the working
    // tree and that row has none.
    context_action(&mut app, 'p');

    assert!(app.status_error);
    assert_eq!(
        app.status,
        "only the current branch can be pulled; check feature out first"
    );
    assert_eq!(provider.pulls(), 1);

    // The changed-file list has no branch rows, so both keys can only mean
    // the branch the working tree is on.
    app.execute_command("git-status").unwrap();
    context_action(&mut app, 'p');
    context_action(&mut app, 'P');

    assert_eq!(provider.pulls(), 2);
    assert_eq!(provider.pushes(), vec!["main", "feature", "main"]);

    fs::remove_dir_all(root).unwrap();
}

/// Two people on one branch: commits here that the remote has not seen and
/// commits there that this clone has not. `p` cannot fast-forward, so it
/// offers to replay the local ones on top rather than reporting a dead end.
#[test]
fn pulling_a_diverged_branch_offers_to_replay_the_local_commits() {
    use crate::git::{Divergence, MemoryGitProvider, Repository, Upstream};

    let root = temporary("git-diverged");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let branches = || {
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["main"], "main")
            .with_branch_detail(
                "main",
                Some(Upstream::origin(
                    "main",
                    Some(Divergence {
                        ahead: 2,
                        behind: 1,
                    }),
                )),
                true,
            )
    };

    // Escape leaves the branch exactly as it was: nothing is replayed, and
    // the drift the offer named is still there to act on later.
    let provider = Rc::new(branches());
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();

    context_action(&mut app, 'p');

    assert!(
        !app.status_error,
        "an offer is not an error: {}",
        app.status
    );
    assert!(app.git_pull_rebase.is_some());
    assert!(app.has_input_overlay(), "the offer owns the next key");
    assert!(app.status.contains("2 local commits"), "{}", app.status);
    assert!(app.status.contains("origin/main"), "{}", app.status);
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Replay commits");
    assert_eq!(overlay.actions[0].label, "replay commits");
    let message = overlay.message.unwrap();
    assert!(message.contains("2 local commits"), "{message}");
    assert!(message.contains("origin/main"), "{message}");
    assert_eq!(provider.pulls(), 0, "nothing was fast-forwarded");
    assert_eq!(provider.rebases(), 0);

    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    assert!(app.git_pull_rebase.is_none());
    assert_eq!(provider.rebases(), 0);
    assert!(app.status.contains("left as it is"), "{}", app.status);
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n* main [↑2 ↓1]\n\nRemote\n  no remote branches known"
    );

    // Enter replays them, and the row afterwards is ahead of an upstream it
    // no longer trails.
    let provider = Rc::new(branches());
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();

    context_action(&mut app, 'p');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.git_pull_rebase.is_none());
    assert_eq!(provider.rebases(), 1);
    assert_eq!(provider.pulls(), 0, "a replay is not a second pull");
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n* main [↑2]\n\nRemote\n  no remote branches known"
    );

    fs::remove_dir_all(root).unwrap();
}

/// A pull rewrites files under open buffers and the editor reloads them, so
/// unwritten edits stop it before the network is reached.
#[test]
fn pulling_refuses_unsaved_buffer_changes_and_reports_a_refusing_remote() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-network-refuse");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("open.rs");
    fs::write(&file, "saved\n").unwrap();
    let root = root.canonicalize().unwrap();
    let file = file.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider =
        Rc::new(MemoryGitProvider::new(Repository::new(&root)).with_branches(&["main"], "main"));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(file).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved "));
    app.execute_command("git-branches").unwrap();

    context_action(&mut app, 'p');

    assert!(app.status_error);
    assert_eq!(app.status, "cannot pull with unsaved file-buffer changes");
    assert_eq!(provider.pulls(), 0);

    fs::remove_dir_all(root).unwrap();

    // A remote that refuses is reported rather than swallowed, and nothing
    // in the editor moves.
    let root = temporary("git-network-unreachable");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["main"], "main")
            .refusing_network(),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();

    context_action(&mut app, 'p');
    assert!(app.status_error, "{}", app.status);
    context_action(&mut app, 'P');
    assert!(app.status_error, "{}", app.status);
    assert!(provider.pushes().is_empty());

    fs::remove_dir_all(root).unwrap();
}

/// Upstream drift reads as a note beside the name: bracketed, lined up in a
/// column, and highlighted apart from the branch it describes.
#[test]
fn the_branch_list_sets_upstream_drift_apart_from_the_name() {
    use crate::git::{MemoryGitProvider, Repository, Upstream};

    let root = temporary("git-branch-drift");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            .with_branch_detail(
                "main",
                Some(Upstream::origin(
                    "main",
                    Some(crate::git::Divergence {
                        ahead: 2,
                        behind: 1,
                    }),
                )),
                true,
            ),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();

    assert_eq!(
        app.active_buffer().to_string(),
        "Local\n  feature\n* main [↑2 ↓1]\n\nRemote\n  no remote branches known"
    );

    // The current branch's annotation is highlighted, and the name before it is
    // not: a reader must be able to tell the two apart at a glance.
    let buffer = app.active().buffer;
    let row = app.active_buffer().line_to_offset(2);
    let end = row + app.active_buffer().line_len(2);
    let spans = app.highlights(buffer, row, end);
    assert_eq!(spans.len(), 1);
    let annotation = app
        .active_buffer()
        .slice(spans[0].from, spans[0].to)
        .to_string();
    assert_eq!(annotation, "[↑2 ↓1]");
    assert_eq!(spans[0].scope.name(), "comment");
    // The first row tracks nothing, so it says nothing and is not
    // highlighted at all.
    let untracked = app.active_buffer().line_to_offset(1);
    assert!(app.highlights(buffer, untracked, untracked + 9).is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn branch_checkout_refuses_git_and_unsaved_buffer_changes() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let root = temporary("git-branch-dirty");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let changed = FileStatus {
        path: PathBuf::from("changed.rs"),
        original_path: None,
        index: FileState::Unmodified,
        worktree: FileState::Modified,
    };
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_status(RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: vec![changed],
            })
            .with_branches(&["feature", "main"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.status_error);
    assert!(app.status.contains("uncommitted changes"), "{}", app.status);
    assert!(provider.checkouts().is_empty());

    fs::remove_dir_all(root).unwrap();

    let root = temporary("git-branch-unsaved");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("open.rs");
    fs::write(&file, "saved\n").unwrap();
    let root = root.canonicalize().unwrap();
    let file = file.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_branches(&["feature", "main"], "main"),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(file).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved "));
    app.execute_command("git-branches").unwrap();
    press(&mut app, 'k');

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.status_error);
    assert!(app.status.contains("unsaved file-buffer changes"));
    assert!(provider.checkouts().is_empty());

    fs::remove_dir_all(root).unwrap();
}

/// Every Git command that works on a file refuses the same three ways.
#[test]
fn git_file_commands_report_why_they_cannot_act() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("git-file-refusals");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(MemoryGitProvider::new(Repository::new(&root))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    // A scratch buffer has no file behind it.
    for command in ["git-diff", "git-stage", "git-unstage"] {
        app.execute_command(command).unwrap();
        assert!(app.status_error, "{command} reported no error");
        assert_eq!(app.status, "this buffer has no file behind it");
    }
    fs::remove_dir_all(root).unwrap();
}

fn git_test_geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    }
}

fn open_commit_detail(body: &str, patch: &str) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_git_commit_detail_result(CommitDetail {
        summary: crate::git::CommitSummary {
            oid: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            abbreviated: "0123456789ab".to_owned(),
            parents: vec!["fedcba9876543210fedcba9876543210fedcba98".to_owned()],
            author: "A Reader".to_owned(),
            author_time: 1_786_687_978,
            author_date: "2026-08-14".to_owned(),
            subject: "subject".to_owned(),
            decorations: Vec::new(),
        },
        body: body.to_owned(),
        patch: patch.to_owned(),
    });
    app
}

#[test]
fn commit_detail_buffers_reuse_full_object_identity() {
    let mut app = open_commit_detail("first", "diff --git a/a b/a\n");
    let first_buffer = app.active().buffer;
    let original_count = app.buffers.len();

    let mut same = match app.buffers[first_buffer].kind.clone() {
        BufferKind::GitCommit { oid, .. } => oid,
        other => panic!("expected commit detail, got {other:?}"),
    };
    app.open_git_commit_detail_result(CommitDetail {
        summary: CommitSummary {
            oid: same.clone(),
            abbreviated: same[..12].to_owned(),
            parents: Vec::new(),
            author: "A Reader".to_owned(),
            author_time: 1,
            author_date: "2026-08-17".to_owned(),
            subject: "refreshed".to_owned(),
            decorations: Vec::new(),
        },
        body: "updated".to_owned(),
        patch: String::new(),
    });
    assert_eq!(app.active().buffer, first_buffer);
    assert_eq!(app.buffers.len(), original_count);

    same.replace_range(..1, "f");
    app.open_git_commit_detail_result(CommitDetail {
        summary: CommitSummary {
            oid: same,
            abbreviated: "f123456789ab".to_owned(),
            parents: Vec::new(),
            author: "A Reader".to_owned(),
            author_time: 2,
            author_date: "2026-08-17".to_owned(),
            subject: "different".to_owned(),
            decorations: Vec::new(),
        },
        body: "other".to_owned(),
        patch: String::new(),
    });
    assert_ne!(app.active().buffer, first_buffer);
    assert_eq!(app.buffers.len(), original_count + 1);
}

#[test]
fn file_end_motions_remain_responsive_in_commit_detail_buffers() {
    let mut app = open_commit_detail("message", "diff --git a/a b/a\n+line\n");
    let end = app.active_buffer().len_chars();

    app.handle_key(KeyStroke::char('G')).unwrap();
    assert_eq!(app.active().head(), end);

    app.handle_key(KeyStroke::char('g')).unwrap();
    app.handle_key(KeyStroke::char('e')).unwrap();
    assert_eq!(app.active().head(), end);
}

fn visible_diff_rows(app: &mut App) -> Vec<(usize, Option<crate::git::DiffLine>)> {
    let prepared = app.prepare_view(git_test_geometry());
    app.snapshot(&prepared)
        .pane(0)
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row {
            crate::snapshot::SnapshotRow::Text(row) => Some((row.document_row, row.diff)),
            crate::snapshot::SnapshotRow::Placeholder
            | crate::snapshot::SnapshotRow::Padding
            | crate::snapshot::SnapshotRow::Filler => None,
        })
        .collect()
}

#[test]
fn commit_detail_colours_only_rows_inside_its_patch_region() {
    let mut app = open_commit_detail(
        "Add new issues\n\n- catppuccin theme\n- everforest theme\n",
        "-removed from the file\n+added to the file\n",
    );
    let text = app.active_buffer().to_string();
    let bullet_row = text
        .lines()
        .position(|line| line == "- catppuccin theme")
        .unwrap();
    let patch_row = text
        .lines()
        .position(|line| line == "-removed from the file")
        .unwrap();
    let rows = visible_diff_rows(&mut app);

    assert_eq!(rows[bullet_row], (bullet_row, None));
    assert_eq!(
        rows[patch_row],
        (patch_row, Some(crate::git::DiffLine::Removed))
    );
    assert_eq!(
        rows[patch_row + 1],
        (patch_row + 1, Some(crate::git::DiffLine::Added))
    );
    assert!(
        rows[..patch_row].iter().all(|(_, diff)| diff.is_none()),
        "commit metadata and message leaked into diff presentation: {rows:?}"
    );
}

#[test]
fn commit_detail_diff_boundary_handles_empty_message_and_empty_patch() {
    let mut with_patch = open_commit_detail("", "-first patch row\n");
    let text = with_patch.active_buffer().to_string();
    let patch_row = text
        .lines()
        .position(|line| line == "-first patch row")
        .unwrap();
    let rows = visible_diff_rows(&mut with_patch);
    assert!(rows[..patch_row].iter().all(|(_, diff)| diff.is_none()));
    assert_eq!(
        rows[patch_row],
        (patch_row, Some(crate::git::DiffLine::Removed)),
        "the exact first row of the patch must be classified"
    );

    let mut without_patch = open_commit_detail("- message only\n", "");
    let rows = visible_diff_rows(&mut without_patch);
    assert!(
        rows.iter().all(|(_, diff)| diff.is_none()),
        "an empty patch has no diff rows: {rows:?}"
    );
}

/// What the list is opened to see: which files changed, and how much.
///
/// The counts reach the buffer through the same status read that produced
/// the rows, so the heading total and the column agree with each other and
/// with the sections between them.
#[test]
fn the_changed_file_list_shows_line_counts_per_file_and_in_total() {
    use crate::git::{
        CountKind, DiffScope, Divergence, FileState, FileStatus, Head, LineStats,
        MemoryGitProvider, Repository, RepositoryStatus,
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-stats-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();

    let file = |name: &str, index, worktree| FileStatus {
        path: PathBuf::from(name),
        original_path: None,
        index,
        worktree,
    };
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = MemoryGitProvider::new(Repository::new(&root))
        .with_status(RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: vec![
                file("staged.rs", FileState::Modified, FileState::Unmodified),
                file("edited.rs", FileState::Unmodified, FileState::Modified),
                file("logo.png", FileState::Unmodified, FileState::Modified),
            ],
        })
        .with_line_stats(DiffScope::Staged, "staged.rs", LineStats::new(82, 12))
        .with_line_stats(DiffScope::Unstaged, "edited.rs", LineStats::new(3, 7));
    ports.replace_git(Box::new(provider));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    app.execute_command("git-status").unwrap();

    assert_eq!(
        app.active_buffer().to_string(),
        "# main · 1 staged · 2 not staged · +85 -19\n\
             \n\
             Staged\n\
             \u{20} M  +82  -12  staged.rs\n\
             \n\
             Not staged\n\
             \u{20} M   +3   -7  edited.rs\n\
             \u{20} M    ·    ·  logo.png"
    );

    // Each count reaches the frame as its own run, saying which of the two
    // it is. Nothing here picks a colour: that is the frontend's to take
    // from the theme, and the same two colours already paint the gutter.
    use crate::snapshot::{SnapshotRow, TextRunKind};

    let geometry = FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 12,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 10,
        },
        status: Rect {
            x: 0,
            y: 10,
            width: 60,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 11,
            width: 60,
            height: 1,
        },
    };
    let prepared = app.prepare_view(geometry);
    let snapshot = app.snapshot(&prepared);
    let counted = |row: usize| {
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[row] else {
            panic!("row {row} is text");
        };
        row.runs
            .iter()
            .filter_map(|run| match run.kind {
                TextRunKind::Text {
                    count: Some(count), ..
                } => Some((run.text.clone(), count)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        counted(3),
        vec![
            ("+82".to_owned(), CountKind::Added),
            ("-12".to_owned(), CountKind::Removed),
        ],
        "the staged row's two numbers, and nothing else on it"
    );
    assert_eq!(
        counted(6),
        vec![
            ("+3".to_owned(), CountKind::Added),
            ("-7".to_owned(), CountKind::Removed),
        ]
    );
    assert!(
        counted(7).is_empty(),
        "a file whose lines were never counted has no number to paint"
    );
    assert!(counted(0).is_empty(), "the heading is not a counted row");

    fs::remove_dir_all(root).unwrap();
}

/// The list is the point at which staging stops being one file at a time:
/// three cursors over three rows stage three files.
#[test]
fn a_selection_over_the_changed_file_list_stages_every_file_it_covers() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-list-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();

    let changed = |name: &str| FileStatus {
        path: PathBuf::from(name),
        original_path: None,
        index: FileState::Unmodified,
        worktree: FileState::Modified,
    };
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut provider =
        MemoryGitProvider::new(Repository::new(&root)).with_status(RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: vec![changed("one.rs"), changed("two.rs"), changed("three.rs")],
        });
    for name in ["one.rs", "two.rs", "three.rs"] {
        provider = provider.with_working(name, "worktree\n");
    }
    ports.replace_git(Box::new(provider));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    app.execute_command("git-status").unwrap();

    let buffer = app.active().buffer;
    assert!(app.buffers[buffer].is_git_status());
    assert!(app.buffers[buffer].is_read_only());
    assert_eq!(app.key_binding_scope(), BindingScope::GitStatus);
    assert_eq!(
        app.buffers[buffer].to_string(),
        "# main · 3 not staged\n\nNot staged\n  M one.rs\n  M two.rs\n  M three.rs"
    );

    // The caret opens on the first file, not on the heading, so a single
    // `s` acts on something.
    assert_eq!(app.buffers[buffer].offset_to_row(app.active().head()), 3);

    // Two of the three rows, selected as a range.
    let first = app.buffers[buffer].line_to_offset(3);
    let last = app.buffers[buffer].line_to_offset(4);
    app.panes
        .get_mut(&app.active_pane)
        .unwrap()
        .replace_selection(Selection::single(Range::new(first, last)));

    app.execute_command("git-stage").unwrap();

    assert_eq!(app.status, "staged 2 paths");
    assert!(!app.status_error);
    // The list rewrote itself, so the two are now on the other side.
    assert_eq!(
        app.buffers[buffer].to_string(),
        "# main · 2 staged · 1 not staged\n\nStaged\n  M one.rs\n  M two.rs\n\nNot staged\n  M three.rs"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stage_all_action_stages_every_unstaged_row_not_just_the_selection() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let root = temporary("git-stage-all");
    fs::create_dir_all(&root).unwrap();
    let changed = |name: &str| FileStatus {
        path: PathBuf::from(name),
        original_path: None,
        index: FileState::Unmodified,
        worktree: FileState::Modified,
    };
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_status(RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: vec![changed("one.rs"), changed("two.rs")],
            })
            .with_working("one.rs", "one\n")
            .with_working("two.rs", "two\n"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-status").unwrap();

    context_action(&mut app, 'S');

    assert_eq!(app.status, "staged 2 paths");
    let text = app.active_buffer().to_string();
    assert!(text.contains("# main · 2 staged"), "{text}");
    assert!(!text.contains("Not staged"), "{text}");

    fs::remove_dir_all(root).unwrap();
}

/// A file staged and then edited again has a row on each side. Selecting
/// both must not act on it twice, and unstaging must still reach it.
#[test]
fn a_file_on_both_sides_is_acted_on_once() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-both-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_status(RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: vec![FileStatus {
                    path: PathBuf::from("both.rs"),
                    original_path: None,
                    index: FileState::Modified,
                    worktree: FileState::Modified,
                }],
            })
            .with_working("both.rs", "worktree\n"),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-status").unwrap();
    let buffer = app.active().buffer;

    // Everything, both rows included.
    let end = app.buffers[buffer].len_chars();
    app.panes
        .get_mut(&app.active_pane)
        .unwrap()
        .replace_selection(Selection::single(Range::new(0, end)));

    app.execute_command("git-unstage").unwrap();

    assert_eq!(
        app.status, "unstaged both.rs",
        "one file, named rather than counted"
    );

    fs::remove_dir_all(root).unwrap();
}

/// Headings and blank rows are not files, and a key pressed on one has to
/// say so rather than reach for a neighbouring row.
#[test]
fn a_heading_row_is_not_a_file() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = temporary_directory().join(format!(
        "runyte-git-heading-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(MemoryGitProvider::new(Repository::new(&root))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-status").unwrap();

    assert_eq!(
        app.active_buffer().to_string(),
        "# main\n\nworking tree clean"
    );

    app.execute_command("git-stage").unwrap();
    assert!(app.status_error);
    assert_eq!(app.status, "no files are selected");

    // Enter is a scoped key rather than a typed command, as it is in the
    // explorer.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.status, "this row is not a file");

    fs::remove_dir_all(root).unwrap();
}

/// Staging moves a file into another section. The caret has to go with it,
/// or the next keypress acts on whichever file closed the gap.
#[test]
fn the_caret_follows_the_file_it_was_on_across_a_refresh() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-follow-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();

    let changed = |name: &str| FileStatus {
        path: PathBuf::from(name),
        original_path: None,
        index: FileState::Unmodified,
        worktree: FileState::Modified,
    };
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut provider =
        MemoryGitProvider::new(Repository::new(&root)).with_status(RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: vec![changed("one.rs"), changed("two.rs"), changed("three.rs")],
        });
    for name in ["one.rs", "two.rs", "three.rs"] {
        provider = provider.with_working(name, "worktree\n");
    }
    ports.replace_git(Box::new(provider));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-status").unwrap();
    let buffer = app.active().buffer;

    // The middle file, so a row-number-preserving caret would land on a
    // different one afterwards.
    let two = app.buffers[buffer].line_to_offset(4);
    app.panes
        .get_mut(&app.active_pane)
        .unwrap()
        .replace_selection(Selection::point(two));

    app.execute_command("git-stage").unwrap();
    assert_eq!(app.status, "staged two.rs");

    let row = app.buffers[buffer].offset_to_row(app.active().head());
    assert_eq!(
        app.buffers[buffer].line_string(row).trim_end(),
        "  M two.rs",
        "the caret stayed with the file it staged"
    );
    // So pressing unstage now undoes exactly what was just done.
    app.execute_command("git-unstage").unwrap();
    assert_eq!(app.status, "unstaged two.rs");

    fs::remove_dir_all(root).unwrap();
}

/// Every action the changed-file list's help advertises, exercised from
/// inside the list through the registry-backed Tab menu.
#[test]
fn every_key_the_changed_file_list_advertises_does_what_it_says() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };
    use crate::help::HelpTopic;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-keys-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    fs::write(root.join("edited.rs"), "worktree\n").unwrap();

    let patch = "@@ -1 +1 @@\n-one\n+two\n";
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_status(RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: vec![FileStatus {
                    path: PathBuf::from("edited.rs"),
                    original_path: None,
                    index: FileState::Unmodified,
                    worktree: FileState::Modified,
                }],
            })
            .with_staged("edited.rs", "one\n")
            .with_working("edited.rs", "worktree\n")
            .with_diff(patch),
    ));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    let open_list = |app: &mut App| {
        app.execute_command("git-status").unwrap();
        assert_eq!(app.key_binding_scope(), BindingScope::GitStatus);
    };
    open_list(&mut app);

    // The help this view opens with is the one being checked.
    assert_eq!(
        HelpTopic::for_context(app.key_binding_scope()),
        HelpTopic::GitStatus
    );

    // Bare search remains search here; only the open menu owns `s` as a
    // staging mnemonic.
    press(&mut app, 's');
    assert_eq!(app.mode, Mode::Command);
    assert!(matches!(app.prompt_kind, PromptKind::Search(_)));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    // The menu snapshot is the same semantic surface an attached client
    // receives: row actions first, then buffer-wide actions.
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::BufferActions)
        .unwrap();
    assert_eq!(
        overlay
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        ["s", "u", "D", "o", "S", "c", "i", "p", "P"]
    );
    // Each row's detail is three columns padded to the widest entry in the
    // menu, so the action word, the context and the sentence all line up
    // however long the neighbouring words are.
    assert_eq!(
        overlay
            .rows
            .iter()
            .map(|row| row.detail.as_str())
            .collect::<Vec<_>>(),
        [
            "stage    row     Stage every file the selection covers",
            "unstage  row     Unstage every file the selection covers",
            "discard  row     Discard every selected file's changes, after a confirmation",
            "open     row     Open the file on this line",
            "stage    buffer  Stage every changed file",
            "commit   buffer  Write a message and commit what is staged",
            "index    buffer  Review everything staged for the next commit",
            "pull     buffer  Fast-forward the current branch onto what it tracks",
            "push     buffer  Publish this branch to what it tracks",
        ]
    );

    // Shift-Tab moves upward with wraparound, and Tab toggles the menu
    // closed. Ctrl-c is the keyboard-wide cancellation spelling.
    key(&mut app, KeyCode::BackTab, Modifiers::SHIFT);
    assert_eq!(
        app.overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::BufferActions)
            .unwrap()
            .selected,
        Some(8)
    );
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.context_action_menu.is_none());
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('c'), Modifiers::CONTROL);
    assert!(app.context_action_menu.is_none());
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    // `Enter` — the row's diff. On an unstaged row that is the unstaged one.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.status_error, "{}", app.status);
    let diff = app.active_buffer().to_string();
    assert_eq!(app.active_buffer().display_name(), "[git diff edited.rs]");
    assert!(diff.starts_with("# not staged · edited.rs"), "{diff}");
    assert!(diff.contains(patch), "{diff}");

    // Arrow/j/k navigation and Enter reach the same actions as mnemonics.
    open_list(&mut app);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    press(&mut app, 'j');
    press(&mut app, 'j');
    press(&mut app, 'j');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(root.join("edited.rs").as_path())
    );

    // `Tab s` — stage, which moves the row to the other section.
    open_list(&mut app);
    context_action(&mut app, 's');
    assert_eq!(app.status, "staged edited.rs");
    assert!(app.active_buffer().to_string().contains("Staged"));

    // Enter on a staged row shows what a commit would take instead.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(
        app.active_buffer()
            .to_string()
            .starts_with("# staged · edited.rs"),
        "{}",
        app.active_buffer()
    );

    // `Tab u` — unstage, back again.
    open_list(&mut app);
    context_action(&mut app, 's');
    context_action(&mut app, 'u');
    assert_eq!(app.status, "unstaged edited.rs");
    assert!(app.active_buffer().to_string().contains("Not staged"));

    // `Space g r` — re-read, and rewrite the list with what was read.
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('r'), Modifiers::NONE);
    assert!(!app.status_error, "{}", app.status);
    assert!(app.status.starts_with("git: main"), "{}", app.status);
    assert!(app.active_buffer().is_git_status(), "the list stayed open");
    assert!(app.active_buffer().to_string().contains("  M edited.rs"));

    // `Tab D` — discard, which asks before it destroys anything.
    open_list(&mut app);
    context_action(&mut app, 'D');
    assert!(
        app.status.contains("Press Enter to discard"),
        "{}",
        app.status
    );
    assert!(app.status.contains("cannot be undone"), "{}", app.status);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.status.contains("cancelled"), "{}", app.status);

    // `Tab i` — the staged review, owned by the list.
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    press(&mut app, 'i');
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.active_buffer().display_name(), "[git index]");

    fs::remove_dir_all(root).unwrap();
}

/// Builds a project whose index holds one staged file, keeping a handle on
/// the fake so a test can ask what it was told.
fn staged_project(name: &str) -> (PathBuf, App, Rc<crate::git::MemoryGitProvider>) {
    staged_project_with(name, |provider| provider)
}

fn staged_project_with(
    name: &str,
    adjust: impl FnOnce(crate::git::MemoryGitProvider) -> crate::git::MemoryGitProvider,
) -> (PathBuf, App, Rc<crate::git::MemoryGitProvider>) {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-git-{name}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(adjust(
        MemoryGitProvider::new(Repository::new(&root)).with_status(RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: vec![FileStatus {
                path: PathBuf::from("lorem.md"),
                original_path: None,
                index: FileState::Modified,
                worktree: FileState::Unmodified,
            }],
        }),
    ));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let app = App::new_in_isolated_project(&root, ports).unwrap();
    (root, app, provider)
}

fn empty_repository_snapshot(
    repository: Repository,
    generation: RepositoryGeneration,
    requested: RefreshSpec,
) -> RepositorySnapshot {
    RepositorySnapshot {
        repository,
        generation,
        started_at: Instant::now(),
        requested,
        status: crate::git::RepositoryStatus {
            head: crate::git::Head::Branch("main".to_owned()),
            upstream: None,
            divergence: crate::git::Divergence::default(),
            files: Vec::new(),
        },
        stats: crate::git::StatusStats::default(),
        head_oid: Some("a".repeat(40)),
        staged: Vec::new(),
        branches: None,
        staged_diff: None,
        file_diffs: Vec::new(),
        worktrees: None,
        log: None,
        requested_log_anchors: Vec::new(),
        reachable_log_anchors: Vec::new(),
        stashes: None,
    }
}

#[test]
fn closing_a_file_retires_its_staged_base() {
    let (root, mut app, _) = staged_project_with("retire-staged-base", |provider| {
        provider.with_staged("closed.rs", "base\n")
    });
    let path = root.join("closed.rs");
    fs::write(&path, "base\n").unwrap();
    app.open_file(path.clone()).unwrap();
    let buffer = app.active().buffer;
    let repository = app.git.repository().unwrap().clone();
    let status = app.git.status().unwrap().clone();
    let generation = app.git_snapshot_generation();
    assert!(app.git.tracks(&path));

    app.close_buffer(buffer);

    assert!(!app.git.tracks(&path));
    app.mark_git_snapshot_stale();
    app.apply_git_response(
        GitOperation::Status {
            repository: repository.clone(),
        },
        GitResponse::Status(status.clone()),
        (None, GitServiceState::Completed),
        RequestedGitViews::default(),
        None,
        None,
    );
    assert!(app.git_state.snapshot_stale());
    app.apply_git_response(
        GitOperation::StagedContent {
            repository: repository.clone(),
            path: path.clone(),
        },
        GitResponse::StagedContent {
            path: path.clone(),
            content: BaseContent::Text("late direct base\n".to_owned()),
        },
        (None, GitServiceState::Completed),
        RequestedGitViews::default(),
        None,
        None,
    );
    assert!(!app.git.tracks(&path));
    assert!(app.git_state.snapshot_stale());
    app.apply_repository_snapshot(
        RepositorySnapshot {
            repository,
            generation,
            started_at: Instant::now(),
            requested: RefreshSpec {
                staged_paths: vec![path.clone()],
                ..RefreshSpec::default()
            },
            status,
            stats: Default::default(),
            head_oid: Some("a".repeat(40)),
            staged: vec![(
                path.clone(),
                BaseContent::Text("late snapshot base\n".to_owned()),
            )],
            branches: None,
            staged_diff: None,
            file_diffs: Vec::new(),
            worktrees: None,
            log: None,
            requested_log_anchors: Vec::new(),
            reachable_log_anchors: Vec::new(),
            stashes: None,
        },
        false,
        false,
    );
    assert!(!app.git.tracks(&path));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_as_retires_the_previous_paths_staged_base() {
    let (root, mut app, _) = staged_project_with("save-as-staged-base", |provider| {
        provider
            .with_staged("before.rs", "base\n")
            .with_staged("after.rs", "base\n")
    });
    let before = root.join("before.rs");
    let after = root.join("after.rs");
    fs::write(&before, "base\n").unwrap();
    app.open_file(before.clone()).unwrap();
    assert!(app.git.tracks(&before));

    app.save(Some(after.clone()), false).unwrap();

    assert!(!app.git.tracks(&before));
    assert!(app.git.tracks(&after));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_as_retains_one_post_write_git_barrier_when_the_queue_is_full() {
    let (root, mut app, _) = staged_project_with("save-as-async-barrier", |provider| {
        provider
            .with_staged("before.rs", "base\n")
            .with_staged("after.rs", "base\n")
    });
    app.config.git.refresh_interval_seconds = 0;
    let before = root.join("before.rs");
    let after = root.join("after.rs");
    fs::write(&before, "base\n").unwrap();
    app.open_file(before.clone()).unwrap();
    assert!(app.git.tracks(&before));
    let buffer = app.active().buffer;
    let end = app.buffers[buffer].len_chars();
    app.buffers[buffer].apply(&Transaction::insert(end, "saved\n"));
    let (service, paused) = GitServiceHandle::saturated_for_test();
    app.attach_git_service(service);

    app.save(Some(after.clone()), false).unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.buffers[buffer].path.as_deref(), Some(after.as_path()));
    assert!(!app.git.tracks(&before));
    assert!(!app.git.tracks(&after));
    assert!(app.git_state.snapshot_stale());
    assert!(!app.retry_pending_git_reconciliation(Instant::now()));
    assert!(matches!(
        paused.next_operation(),
        GitOperation::Discover { .. }
    ));
    assert!(app.retry_pending_git_reconciliation(Instant::now()));
    let mut ordinary_staged_read = false;
    let reconciliation = loop {
        match paused.next_operation() {
            GitOperation::StagedContent { .. } => ordinary_staged_read = true,
            GitOperation::Reconcile { spec, .. } => break spec,
            _ => {}
        }
    };
    assert!(
        !ordinary_staged_read,
        "save-as submitted a coalescible staged-content read"
    );
    assert_eq!(reconciliation.staged_paths, vec![after]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_explorer_move_reconciles_git_with_monitoring_disabled() {
    let (root, mut app, provider) = staged_project_with("explorer-move-git", |provider| {
        provider
            .with_staged("before.rs", "base\n")
            .with_staged("after.rs", "base\n")
    });
    app.config.git.refresh_interval_seconds = 0;
    let before = root.join("before.rs");
    let after = root.join("after.rs");
    fs::write(&before, "base\n").unwrap();
    app.open_file(before.clone()).unwrap();
    let file = app.active().buffer;
    assert!(app.git.tracks(&before));
    let calls_before = provider.calls();
    fs::rename(&before, &after).unwrap();
    provider.set_status(crate::git::RepositoryStatus {
        head: crate::git::Head::Branch("after-move".to_owned()),
        upstream: None,
        divergence: crate::git::Divergence::default(),
        files: Vec::new(),
    });
    let report = ApplyReport {
        applied: vec![FsOperation::Rename {
            from: PathBuf::from("before.rs"),
            to: PathBuf::from("after.rs"),
            kind: EntryKind::File,
        }],
    };

    assert_eq!(
        app.reconcile_applied_filesystem(&root, file, &report, true),
        None
    );

    assert_eq!(app.buffers[file].path.as_deref(), Some(after.as_path()));
    assert!(!app.git.tracks(&before));
    assert!(app.git.tracks(&after));
    assert!(provider.calls() > calls_before);
    assert_eq!(
        app.git.status().unwrap().head,
        crate::git::Head::Branch("after-move".to_owned())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_partial_explorer_report_retries_one_async_post_change_barrier() {
    let (root, mut app, _) = staged_project_with("explorer-move-async-barrier", |provider| {
        provider.with_staged("before.rs", "base\n")
    });
    app.config.git.refresh_interval_seconds = 0;
    let before = root.join("before.rs");
    let after = root.join("after.rs");
    fs::write(&before, "base\n").unwrap();
    app.open_file(before.clone()).unwrap();
    let file = app.active().buffer;
    assert!(app.git.tracks(&before));
    let (service, paused) = GitServiceHandle::saturated_for_test();
    app.attach_git_service(service);
    fs::rename(&before, &after).unwrap();
    let report = ApplyReport {
        applied: vec![FsOperation::Rename {
            from: PathBuf::from("before.rs"),
            to: PathBuf::from("after.rs"),
            kind: EntryKind::File,
        }],
    };

    assert_eq!(
        app.reconcile_applied_filesystem(&root, file, &report, false),
        None
    );

    assert_eq!(app.buffers[file].path.as_deref(), Some(after.as_path()));
    assert!(!app.git.tracks(&before));
    assert!(!app.git.tracks(&after));
    assert!(!app.retry_pending_git_reconciliation(Instant::now()));
    assert!(matches!(
        paused.next_operation(),
        GitOperation::Discover { .. }
    ));
    assert!(app.retry_pending_git_reconciliation(Instant::now()));
    let mut staged_read = false;
    let reconciliation = loop {
        match paused.next_operation() {
            GitOperation::StagedContent { .. } => staged_read = true,
            GitOperation::Reconcile { spec, .. } => break spec,
            _ => {}
        }
    };
    assert!(!staged_read, "retargeted files were submitted one by one");
    assert_eq!(reconciliation.staged_paths, vec![after]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_pre_change_snapshot_cannot_mark_an_inflight_filesystem_barrier_fresh() {
    let root = temporary("explorer-barrier-stale-marker");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let path = root.join("moved.rs");
    fs::write(&path, "base\n").unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_file(path.clone()).unwrap();
    app.git.attach(Some(repository.clone()));
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    assert!(matches!(
        operations.recv_timeout(Duration::from_secs(1)).unwrap(),
        GitOperation::Discover { .. }
    ));

    app.reconcile_git_after_filesystem(vec![path]);
    let reconciliation = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    let reconciliation_spec = match &reconciliation {
        GitOperation::Reconcile { spec, .. } => spec.clone(),
        _ => panic!("filesystem change did not submit its ordering barrier"),
    };
    assert!(app.git_state.snapshot_stale());

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(99),
        operation: GitOperation::Refresh {
            repository: repository.clone(),
            spec: RefreshSpec::default(),
        },
        result: Box::new(Ok(GitResponse::Snapshot(Box::new(
            empty_repository_snapshot(
                repository.clone(),
                RepositoryGeneration::default(),
                RefreshSpec::default(),
            ),
        )))),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(
        app.git_state.snapshot_stale(),
        "the pre-change read hid the pending barrier"
    );

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(2),
        operation: reconciliation,
        result: Box::new(Ok(GitResponse::Snapshot(Box::new(
            empty_repository_snapshot(
                repository,
                RepositoryGeneration::default(),
                reconciliation_spec,
            ),
        )))),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(!app.git_state.snapshot_stale());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failed_filesystem_barrier_retains_its_spec_for_a_bounded_retry() {
    let root = temporary("explorer-barrier-failure-retry");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let path = root.join("moved.rs");
    fs::write(&path, "base\n").unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_file(path.clone()).unwrap();
    app.git.attach(Some(repository));
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    operations.recv_timeout(Duration::from_secs(1)).unwrap();
    app.reconcile_git_after_filesystem(vec![path.clone()]);
    let failed = operations.recv_timeout(Duration::from_secs(1)).unwrap();

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(2),
        operation: failed,
        result: Box::new(Err(crate::git::GitError::Failed {
            command: "refresh Git".to_owned(),
            code: Some(1),
            signal: None,
            stderr: "transient failure".to_owned(),
        })),
        state: GitServiceState::Failed,
        coalesced: false,
    });

    assert!(app.git_state.snapshot_stale());
    assert!(
        !app.retry_pending_git_reconciliation(Instant::now()),
        "a failed worker must not spin an immediate retry loop"
    );
    assert!(app.retry_pending_git_reconciliation(Instant::now() + Duration::from_secs(2)));
    let retried = (0..2)
        .find_map(
            |_| match operations.recv_timeout(Duration::from_secs(1)).unwrap() {
                GitOperation::Reconcile { spec, .. } => Some(spec),
                _ => None,
            },
        )
        .expect("failed filesystem barrier was not retried");
    assert!(retried.staged_paths.contains(&path));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explorer_moves_outside_git_boundaries_are_not_batched_as_staged_reads() {
    let root = temporary("explorer-move-git-boundaries");
    let repository_root = root.join("repository");
    let workspace = repository_root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let root = root.canonicalize().unwrap();
    let repository_root = repository_root.canonicalize().unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let first = workspace.join("first.rs");
    let second = workspace.join("second.rs");
    let outside_workspace = repository_root.join("outside-workspace.rs");
    let outside_repository = root.join("outside-repository.rs");
    fs::write(&first, "first\n").unwrap();
    fs::write(&second, "second\n").unwrap();
    let repository = Repository::new(&repository_root);
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        crate::git::MemoryGitProvider::new(repository)
            .with_staged("workspace/first.rs", "first\n")
            .with_staged("workspace/second.rs", "second\n"),
    ));
    let mut app = App::new_in_isolated_project(&workspace, ports).unwrap();
    app.open_file(first.clone()).unwrap();
    let first_buffer = app.active().buffer;
    app.open_file(second.clone()).unwrap();
    let second_buffer = app.active().buffer;
    assert!(app.git.tracks(&first));
    assert!(app.git.tracks(&second));
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    operations.recv_timeout(Duration::from_secs(1)).unwrap();
    fs::rename(&first, &outside_workspace).unwrap();
    fs::rename(&second, &outside_repository).unwrap();
    let report = ApplyReport {
        applied: vec![
            FsOperation::Rename {
                from: first,
                to: outside_workspace.clone(),
                kind: EntryKind::File,
            },
            FsOperation::Rename {
                from: second,
                to: outside_repository.clone(),
                kind: EntryKind::File,
            },
        ],
    };

    app.reconcile_applied_filesystem(&workspace, second_buffer, &report, true);

    assert_eq!(
        app.buffers[first_buffer].path.as_deref(),
        Some(outside_workspace.as_path())
    );
    assert_eq!(
        app.buffers[second_buffer].path.as_deref(),
        Some(outside_repository.as_path())
    );
    let GitOperation::Reconcile { spec, .. } =
        operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("explorer move did not submit its Git reconciliation");
    };
    assert!(spec.staged_paths.is_empty());
    fs::remove_dir_all(root).unwrap();
}

/// The template answers "what am I committing" and "how do I finish"
/// without the reader leaving the buffer.
#[test]
fn a_commit_message_opens_on_an_empty_first_line_above_what_it_will_record() {
    let (root, mut app, _) = staged_project("commit-open");

    app.execute_command("git-commit").unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.active_buffer().display_name(), "[git commit]");
    assert!(app.active_buffer().is_commit_message());
    assert!(!app.active_buffer().is_read_only());
    assert_eq!(
        app.mode,
        Mode::Insert,
        "the caret starts where the message goes"
    );
    assert_eq!(app.cursor_position(), Position::new(0, 0));
    let text = app.active_buffer().to_string();
    assert!(text.starts_with('\n'), "{text}");
    assert!(text.contains("# Write this buffer to commit"), "{text}");
    assert!(text.contains("# On main"), "{text}");
    assert!(text.contains("#   M lorem.md"), "{text}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn writing_the_message_commits_the_index_and_closes_the_buffer() {
    let (root, mut app, provider) = staged_project("commit-write");
    app.execute_command("git-status").unwrap();
    app.execute_command("git-commit").unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(
        0,
        "Reword the heading\n\nWith a body.",
    ));

    app.save(None, false).unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert!(app.status.starts_with("[main "), "{}", app.status);
    assert!(
        app.closed_buffers.contains(&buffer),
        "the buffer stayed open"
    );
    assert!(app.active_buffer().is_git_status());
    // The comments are not part of what was recorded, and the body is.
    assert_eq!(
        provider.commits(),
        vec!["Reword the heading\n\nWith a body.".to_owned()]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn write_quit_commits_without_quitting_from_a_commit_message() {
    let (root, mut app, provider) = staged_project("commit-write-quit");
    app.execute_command("git-status").unwrap();
    let origin = app.active().buffer;
    app.execute_command("git-commit").unwrap();
    app.insert_text("Keep the editor open");

    app.execute_command("wq").unwrap();

    assert_eq!(provider.commits(), vec!["Keep the editor open".to_owned()]);
    assert!(!app.closed_buffers.contains(&origin));
    assert_eq!(app.active().buffer, origin);
    assert!(!app.should_quit);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quitting_an_edited_commit_message_requires_force_and_never_commits() {
    let (root, mut app, provider) = staged_project("commit-quit");
    app.execute_command("git-commit").unwrap();
    let commit = app.active().buffer;
    app.insert_text("Do not commit this");

    app.execute_command("q").unwrap();
    assert_eq!(app.active().buffer, commit);
    assert!(!app.should_quit);
    assert!(app.status.contains(":q!"), "{}", app.status);

    app.execute_command("q!").unwrap();
    assert!(app.closed_buffers.contains(&commit));
    assert!(app.should_quit);
    assert!(provider.commits().is_empty());

    fs::remove_dir_all(root).unwrap();
}

/// An empty message is the one mistake that must not reach Git, because
/// Git would refuse it with a message about the editor rather than about
/// what the person did.
#[test]
fn an_empty_message_refuses_and_keeps_the_buffer() {
    let (root, mut app, provider) = staged_project("commit-empty");
    app.execute_command("git-commit").unwrap();
    let buffer = app.active().buffer;

    app.save(None, false).unwrap();

    assert!(app.status_error);
    assert!(app.status.contains("needs a message"), "{}", app.status);
    assert_eq!(app.active().buffer, buffer, "the template was thrown away");
    assert!(provider.commits().is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nothing_staged_is_refused_before_a_message_is_written() {
    use crate::git::{MemoryGitProvider, Repository};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = temporary_directory().join(format!(
        "runyte-git-nothing-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(MemoryGitProvider::new(Repository::new(&root))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    app.execute_command("git-commit").unwrap();

    assert!(app.status_error);
    assert_eq!(app.status, "nothing is staged for commit");
    assert!(!app.active_buffer().is_commit_message());

    fs::remove_dir_all(root).unwrap();
}

/// A rejected hook or an unset identity is something to fix and retry.
/// Losing the message to it would be the worst possible response.
#[test]
fn a_refused_commit_keeps_the_message() {
    let (root, mut app, provider) = staged_project_with(
        "commit-refused",
        crate::git::MemoryGitProvider::refusing_commits,
    );
    app.execute_command("git-commit").unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "A message worth keeping"));

    app.save(None, false).unwrap();

    assert!(app.status_error);
    assert_eq!(app.active().buffer, buffer);
    assert!(
        app.buffers[buffer]
            .to_string()
            .starts_with("A message worth keeping"),
        "the message was lost"
    );
    assert!(provider.commits().is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn comment_lines_are_not_part_of_the_message() {
    assert_eq!(
        commit_message_body("\n\nSubject\n\nBody line\n# a comment\n#   M file.rs\n"),
        "Subject\n\nBody line"
    );
    assert_eq!(commit_message_body("\n# only comments\n"), "");
    assert_eq!(commit_message_body("   \n"), "");
}

/// Buffer closure changes what panes show, never the pane layout itself.
#[test]
fn closing_a_buffer_keeps_every_pane_and_selects_the_next_buffer() {
    let directory = temporary("close-buffer");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first.txt");
    let second = directory.join("second.txt");
    fs::write(&first, "one\n").unwrap();
    fs::write(&second, "two\n").unwrap();

    let mut app = App::new(Config::default(), Some(first.clone())).unwrap();
    app.execute_command("vsplit").unwrap();
    app.execute_command(&format!("open {}", second.display()))
        .unwrap();
    assert_eq!(app.panes.len(), 2);
    let closing = app.active().buffer;

    app.execute_command("bc").unwrap();

    assert!(app.closed_buffers.contains(&closing));
    assert_eq!(app.panes.len(), 2, "buffer closure changed the layout");
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(first.as_path()),
        "the next live buffer wraps around the buffer arena"
    );

    // With nothing else open, every pane showing the buffer gets the same
    // newly-created scratch replacement.
    app.execute_command("bc").unwrap();
    assert_eq!(app.panes.len(), 2);
    assert!(
        app.active_buffer().path.is_none(),
        "nothing else was open, so a scratch buffer took over"
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn closing_a_shared_buffer_retargets_every_view_without_closing_one() {
    let mut app = App::new(Config::default(), None).unwrap();
    let closing = app.active().buffer;
    app.execute_command("vsplit").unwrap();

    app.execute_command("bc").unwrap();

    assert_eq!(app.panes.len(), 2);
    assert!(app.closed_buffers.contains(&closing));
    let shown = app
        .panes
        .values()
        .map(|pane| pane.buffer)
        .collect::<HashSet<_>>();
    assert_eq!(shown.len(), 1, "the views received different fallbacks");
    assert!(app.active_buffer().path.is_none());
}

#[test]
fn closing_a_shared_buffer_uses_each_panes_own_recent_history() {
    let directory = temporary("close-pane-mru");
    fs::create_dir_all(&directory).unwrap();
    let paths = ["a.txt", "b.txt", "c.txt", "d.txt"].map(|name| directory.join(name));
    for path in &paths {
        fs::write(path, path.file_name().unwrap().to_string_lossy().as_bytes()).unwrap();
    }

    let mut app = App::new(Config::default(), Some(paths[0].clone())).unwrap();
    app.open_file(paths[1].clone()).unwrap();
    let shared = app.active().buffer;
    app.split(Axis::Horizontal, None).unwrap();
    let second = app.active_pane;
    app.open_file(paths[2].clone()).unwrap();
    let second_previous = app.active().buffer;
    app.switch_buffer(shared);

    let first = app
        .panes
        .keys()
        .copied()
        .find(|pane| *pane != second)
        .unwrap();
    app.activate_pane(first);
    app.open_file(paths[3].clone()).unwrap();
    let first_previous = app.active().buffer;
    app.switch_buffer(shared);

    app.execute_command("close").unwrap();

    assert!(app.closed_buffers.contains(&shared));
    assert_eq!(app.panes[&first].buffer, first_previous);
    assert_eq!(app.panes[&second].buffer, second_previous);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn an_empty_clean_scratch_buffer_retires_after_its_last_view_leaves() {
    let directory = temporary("ephemeral-buffers");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.txt");
    fs::write(&path, "note").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    let scratch = app.active().buffer;
    app.execute_command(&format!("open {}", path.display()))
        .unwrap();
    assert!(app.closed_buffers.contains(&scratch));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_two_most_recent_clean_special_buffers_remain_jumpable() {
    let mut app = App::new(Config::default(), None).unwrap();

    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let help = app.active().buffer;
    app.execute_command("help").unwrap();
    let manual = app.active().buffer;

    assert!(!app.closed_buffers.contains(&help));
    assert!(!app.closed_buffers.contains(&manual));
    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);
    assert_eq!(app.active().buffer, help);
    key(&mut app, KeyCode::Char('i'), Modifiers::ALT);
    assert_eq!(app.active().buffer, manual);
}

#[test]
fn retained_clean_special_buffers_remain_discoverable_after_their_panes_leave() {
    let directory = temporary("retained-special-buffer-picker");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("ordinary.txt");
    fs::write(&path, "ordinary\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();

    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let contextual_help = app.active().buffer;
    app.execute_command("help").unwrap();
    let manual = app.active().buffer;
    app.open_file(path).unwrap();
    app.retire_detached_ephemeral_buffers();

    assert!(!app.closed_buffers.contains(&contextual_help));
    assert!(!app.closed_buffers.contains(&manual));
    app.open_buffer_picker();
    assert!(
        app.list_actions.iter().any(
            |action| matches!(action, ListAction::Buffer(buffer) if *buffer == contextual_help)
        )
    );
    assert!(
        app.list_actions
            .iter()
            .any(|action| matches!(action, ListAction::Buffer(buffer) if *buffer == manual))
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opening_one_clean_special_buffer_past_the_limit_retires_the_least_recent_detached_one() {
    let mut app = App::new(Config::default(), None).unwrap();

    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let help = app.active().buffer;
    app.execute_command("help").unwrap();
    let manual = app.active().buffer;

    // Revisiting contextual help makes the manual the least recent view.
    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);
    assert_eq!(app.active().buffer, help);
    // Fill the rest of the limit with pages newer than both, so the manual is
    // still the one the next activation has to give up.
    let filler = open_filler_special_buffers(&mut app, SPECIAL_BUFFER_RETENTION_LIMIT - 2);
    app.execute_command("config").unwrap();
    let config = app.active().buffer;

    assert!(app.closed_buffers.contains(&manual));
    assert!(!app.closed_buffers.contains(&help));
    assert!(!app.closed_buffers.contains(&config));
    for buffer in &filler {
        assert!(!app.closed_buffers.contains(buffer));
    }
    assert_eq!(
        app.buffers
            .iter()
            .enumerate()
            .filter(|(index, buffer)| {
                !app.closed_buffers.contains(index) && buffer.is_special() && !buffer.dirty
            })
            .count(),
        SPECIAL_BUFFER_RETENTION_LIMIT
    );

    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);
    assert_eq!(
        app.active().buffer,
        filler.last().copied().unwrap_or(help),
        "history left the view the retained set was entered from"
    );
}

#[test]
fn an_async_special_view_precedes_the_buffer_reached_by_immediate_history_navigation() {
    let mut app = App::new(Config::default(), None).unwrap();

    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let help = app.active().buffer;
    app.execute_command("help").unwrap();
    let manual = app.active().buffer;

    // Git results land outside the input and semantic-command boundaries
    // that run special-buffer retirement. Model that real asynchronous
    // activation path, then return immediately through history.
    app.open_git_worktrees_result(
        vec![Worktree {
            path: PathBuf::from("/worktree"),
            head: Some("0123456789abcdef".to_owned()),
            branch: Some("refs/heads/main".to_owned()),
            detached: false,
            bare: false,
            locked: None,
            prunable: None,
            missing: false,
            common_dir: PathBuf::from("/git"),
        }],
        true,
    );
    let worktrees = app.active().buffer;
    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);
    assert_eq!(app.active().buffer, manual);

    // Fill the rest of the limit only now, so the three views under test keep
    // the relative order the asynchronous activation gave them: contextual
    // help is the oldest, and the worktrees view is older than the manual the
    // immediate history jump returned to.
    open_filler_special_buffers(&mut app, SPECIAL_BUFFER_RETENTION_LIMIT - 2);
    app.retire_detached_ephemeral_buffers();
    assert!(app.closed_buffers.contains(&help));
    assert!(!app.closed_buffers.contains(&worktrees));

    app.execute_command("notifications").unwrap();
    let notifications = app.active().buffer;
    assert!(app.closed_buffers.contains(&worktrees));
    assert!(!app.closed_buffers.contains(&manual));
    assert!(!app.closed_buffers.contains(&notifications));
}

#[cfg(unix)]
#[test]
fn an_empty_scratch_behind_a_terminal_retires_when_a_file_can_replace_it() {
    let directory = temporary("terminal-scratch-lifetime");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.txt");
    fs::write(&path, "note").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    let scratch = app.active().buffer;
    app.open_file(path).unwrap();
    let file = app.active().buffer;
    app.switch_buffer(scratch);
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();

    app.retire_detached_ephemeral_buffers();

    assert!(app.closed_buffers.contains(&scratch));
    assert_eq!(app.active().buffer, file);
    assert_eq!(app.active_terminal(), Some(terminal));
    app.close_terminal_id(terminal);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_dirty_special_buffer_remains_discoverable_after_its_last_view_leaves() {
    let directory = temporary("dirty-special-lifetime");
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("note.txt");
    fs::write(&file, "note").unwrap();
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let explorer = app.active().buffer;
    app.buffers[explorer].apply(&Transaction::insert(0, "planned.txt\n"));

    app.execute_command(&format!("open {}", file.display()))
        .unwrap();
    assert!(!app.closed_buffers.contains(&explorer));
    app.open_buffer_picker();
    assert!(
        app.list_actions
            .iter()
            .any(|action| matches!(action, ListAction::Buffer(buffer) if *buffer == explorer))
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn quit_closes_an_exclusive_buffer_but_keeps_a_shared_one() {
    let directory = temporary("quit-buffer-ownership");
    fs::create_dir_all(&directory).unwrap();
    let first_path = directory.join("first.txt");
    let second_path = directory.join("second.txt");
    fs::write(&first_path, "first").unwrap();
    fs::write(&second_path, "second").unwrap();

    let mut app = App::new(Config::default(), Some(first_path)).unwrap();
    let shared = app.active().buffer;
    app.execute_command("vsplit").unwrap();
    app.execute_command(&format!("open {}", second_path.display()))
        .unwrap();
    let exclusive = app.active().buffer;
    app.execute_command("q").unwrap();
    assert_eq!(app.panes.len(), 1);
    assert!(app.closed_buffers.contains(&exclusive));
    assert!(!app.closed_buffers.contains(&shared));

    app.execute_command("vsplit").unwrap();
    app.execute_command("q").unwrap();
    assert_eq!(app.panes.len(), 1);
    assert!(!app.closed_buffers.contains(&shared));
    fs::remove_dir_all(directory).unwrap();
}

/// The bound safe form refuses unsaved text; only the typed force command
/// can discard it.
#[test]
fn closing_a_modified_buffer_requires_the_force_command() {
    let directory = temporary("close-modified");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("edited.txt");
    fs::write(&path, "one\n").unwrap();

    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved "));

    app.execute_command("bc").unwrap();
    assert!(app.status.contains(":close!"), "{}", app.status);
    assert!(!app.closed_buffers.contains(&buffer));
    assert!(app.buffers[buffer].dirty);

    app.execute_command("bc!").unwrap();
    assert!(app.closed_buffers.contains(&buffer));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "one\n",
        "closing must not write"
    );

    fs::remove_dir_all(directory).unwrap();
}

/// The instruction the commit template gives has to be one that works.
#[test]
fn closing_a_commit_message_abandons_it_and_stages_nothing_differently() {
    let (root, mut app, provider) = staged_project("commit-cancel");
    app.execute_command("git-status").unwrap();
    let list = app.active().buffer;
    app.execute_command("git-commit").unwrap();
    let commit = app.active().buffer;
    app.buffers[commit].apply(&Transaction::insert(0, "Half a thought"));

    app.execute_command("bc").unwrap();
    assert!(app.status.contains(":close!"), "{}", app.status);
    assert!(!app.closed_buffers.contains(&commit));
    app.execute_command("bc!").unwrap();

    assert!(app.closed_buffers.contains(&commit));
    assert!(provider.commits().is_empty(), "nothing was committed");
    assert!(app.status.contains("commit cancelled"), "{}", app.status);
    assert!(!app.closed_buffers.contains(&list));
    assert_eq!(app.active().buffer, list);

    fs::remove_dir_all(root).unwrap();
}

/// Explorer buffers obey the same buffer-local close semantics as text.
#[test]
fn closing_an_explorer_preserves_its_pane() {
    let directory = temporary("close-explorer");
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("kept.txt");
    fs::write(&file, "one\n").unwrap();

    let mut app = App::new(Config::default(), Some(file.clone())).unwrap();
    app.execute_command("vsplit").unwrap();
    app.execute_command(&format!("open {}", directory.display()))
        .unwrap();
    assert!(app.active_buffer().is_directory());
    let explorer = app.active().buffer;
    assert_eq!(app.panes.len(), 2);

    app.execute_command("bc").unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.panes.len(), 2);
    assert!(app.closed_buffers.contains(&explorer));
    assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));

    fs::remove_dir_all(directory).unwrap();
}

/// With one pane the explorer gives way to the next live buffer.
#[test]
fn closing_the_last_explorer_shows_another_buffer() {
    let directory = temporary("close-explorer-last");
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("kept.txt");
    fs::write(&file, "one\n").unwrap();

    let mut app = App::new(Config::default(), Some(file.clone())).unwrap();
    app.execute_command(&format!("open {}", directory.display()))
        .unwrap();
    assert!(app.active_buffer().is_directory());
    let explorer = app.active().buffer;

    app.execute_command("bc").unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));
    assert!(app.closed_buffers.contains(&explorer));

    fs::remove_dir_all(directory).unwrap();
}

/// Unapplied explorer edits require the force spelling, which drops the
/// plan without touching the filesystem.
#[test]
fn closing_an_edited_explorer_requires_force_and_touches_no_files() {
    let directory = temporary("close-explorer-edited");
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("kept.txt");
    fs::write(&file, "one\n").unwrap();

    let mut app = App::new(Config::default(), Some(file.clone())).unwrap();
    app.execute_command(&format!("open {}", directory.display()))
        .unwrap();
    let explorer = app.active().buffer;
    app.buffers[explorer].apply(&Transaction::insert(0, "invented.txt\n"));
    assert!(app.buffers[explorer].dirty);

    app.execute_command("bc").unwrap();
    assert!(app.status.contains(":close!"), "{}", app.status);
    assert!(!app.closed_buffers.contains(&explorer));
    app.execute_command("bc!").unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert!(!app.active_buffer().is_directory());
    assert!(
        !directory.join("invented.txt").exists(),
        "closing must not apply a filesystem plan"
    );
    assert!(file.exists());

    fs::remove_dir_all(directory).unwrap();
}

/// Discarding restores the file and the buffer showing it, because the
/// file changed underneath and a stale buffer would refuse its next save.
#[test]
fn discarding_restores_the_file_and_reloads_its_buffer() {
    let (root, mut app, provider) = staged_project("discard");
    let path = root.join("lorem.md");
    fs::write(&path, "edited on disk\n").unwrap();
    app.open_file(path.clone()).unwrap();
    let buffer = app.active().buffer;
    assert_eq!(app.buffers[buffer].to_string(), "edited on disk\n");

    app.execute_command("git-discard").unwrap();
    assert!(
        app.status.contains("Press Enter to discard"),
        "{}",
        app.status
    );
    assert!(app.status.contains("lorem.md"), "{}", app.status);
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Discard Git changes");
    assert_eq!(overlay.actions[0].label, "discard changes");
    let message = overlay.message.unwrap();
    assert!(message.contains("lorem.md"), "{message}");
    assert!(message.contains("cannot be undone"), "{message}");
    // Nothing happens until the second key.
    assert!(provider.discards().is_empty());

    // The fake restores by truncating; the point is that the buffer follows.
    fs::write(&path, "committed\n").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(!app.status_error, "{}", app.status);
    assert_eq!(provider.discards(), vec![PathBuf::from("lorem.md")]);
    assert_eq!(app.status, "discarded changes to lorem.md");
    assert_eq!(
        app.buffers[buffer].to_string(),
        "committed\n",
        "the buffer kept showing text the file no longer had"
    );
    assert!(!app.buffers[buffer].dirty);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn synchronous_discard_closes_a_removed_staged_addition_buffer() {
    let root = temporary("discard-added-buffer");
    fs::create_dir_all(&root).unwrap();
    let run = |arguments: &[&str]| {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["init", "-q"]);
    run(&["config", "user.name", "Runyte Test"]);
    run(&["config", "user.email", "runyte@example.invalid"]);
    fs::write(root.join("base.rs"), "base\n").unwrap();
    run(&["add", "base.rs"]);
    run(&["commit", "-qm", "base"]);
    let added = root.join("added.rs");
    fs::write(&added, "added\n").unwrap();
    run(&["add", "added.rs"]);

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(crate::git::GitCliProvider::new("git")));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    assert!(
        app.git.repository().is_some(),
        "repository discovery did not attach"
    );
    app.open_file(added.clone()).unwrap();
    let buffer = app.active().buffer;

    app.execute_command("git-discard").unwrap();
    assert!(
        app.git_discard_confirmation.is_some(),
        "discard confirmation was not prepared: {}",
        app.status
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(!app.status_error, "{}", app.status);
    assert!(!added.exists(), "discard kept the staged addition on disk");
    assert!(
        app.closed_buffers.contains(&buffer),
        "the removed file kept an open buffer which could recreate it"
    );
    fs::remove_dir_all(root).unwrap();
}

/// Escape must leave every file exactly as it was.
#[test]
fn cancelling_a_discard_changes_nothing() {
    let (root, mut app, provider) = staged_project("discard-cancel");
    let path = root.join("lorem.md");
    fs::write(&path, "edited\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("git-discard").unwrap();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    assert_eq!(app.status, "discard cancelled; nothing was changed");
    assert!(provider.discards().is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), "edited\n");

    fs::remove_dir_all(root).unwrap();
}

/// Disk restoration never overwrites text that exists only in a buffer.
#[test]
fn a_discard_refuses_unwritten_edits() {
    let (root, mut app, _) = staged_project("discard-unsaved");
    let path = root.join("lorem.md");
    fs::write(&path, "on disk\n").unwrap();
    app.open_file(path).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unwritten "));

    app.execute_command("git-discard").unwrap();

    assert!(app.status_error, "{}", app.status);
    assert!(app.status.contains("unsaved changes"), "{}", app.status);
    assert!(app.git_discard_confirmation.is_none());

    fs::remove_dir_all(root).unwrap();
}

/// An untracked file has no committed version to go back to, so discarding
/// it could only mean deleting it — which belongs in the explorer, where
/// deletion is a confirmed plan that lands in the trash.
#[test]
fn untracked_files_are_refused_rather_than_deleted() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, Repository, RepositoryStatus,
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = temporary_directory().join(format!(
        "runyte-git-discard-untracked-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let root = directory.canonicalize().unwrap();
    let stray = root.join("stray.rs");
    fs::write(&stray, "never tracked\n").unwrap();

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(MemoryGitProvider::new(Repository::new(&root)).with_status(
        RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: vec![FileStatus {
                path: PathBuf::from("stray.rs"),
                original_path: None,
                index: FileState::Untracked,
                worktree: FileState::Untracked,
            }],
        },
    ));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(stray.clone()).unwrap();

    app.execute_command("git-discard").unwrap();

    assert!(app.status_error);
    assert!(
        app.status.contains("no committed version"),
        "{}",
        app.status
    );
    assert!(app.git_discard_confirmation.is_none());
    assert!(provider.discards().is_empty());
    assert!(stray.exists(), "an untracked file was deleted");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_refreshed_diff_follows_the_same_line_in_the_same_hunk() {
    let before = "# diff\n\n@@ -1,2 +1,2 @@\n-old\n+new\n context\n";
    let after = "# diff\n# refreshed\n\n@@ -1,2 +1,2 @@\n-old\n+new\n context\n";
    let identity = diff_row_identity(before, 4).unwrap();

    assert_eq!(diff_row_for_identity(after, &identity), Some(5));
}

#[test]
fn async_refresh_requests_staged_bases_only_for_visible_open_files() {
    let (root, mut app, _) = staged_project("async-open-bases");
    let first = root.join("first.rs");
    let second = root.join("second.rs");
    fs::write(&first, "first\n").unwrap();
    fs::write(&second, "second\n").unwrap();
    app.open_file(root.clone()).unwrap();
    app.open_file(first.clone()).unwrap();
    app.open_file(second.clone()).unwrap();

    let repository = app.git.repository().unwrap().clone();
    let spec = app.git_refresh_spec(&repository);

    assert!(spec.staged_paths.contains(&second));
    assert!(
        !spec.staged_paths.contains(&first),
        "a hidden file caused an unnecessary staged-content read"
    );
    assert!(
        !spec.staged_paths.contains(&root),
        "a directory buffer became an empty Git pathspec"
    );

    let active_pane = app.active_pane;
    app.panes.get_mut(&active_pane).unwrap().terminal = Some(TerminalId::from_raw(1));
    assert!(!app.has_visible_git_state());
    assert!(app.git_refresh_spec(&repository).staged_paths.is_empty());
    app.panes.get_mut(&active_pane).unwrap().terminal = None;

    app.split(Axis::Horizontal, Some(first.clone())).unwrap();
    app.toggle_maximized(MaximizedView::Fullscreen);
    let maximized = app.git_refresh_spec(&repository);
    assert_eq!(maximized.staged_paths, vec![first]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_rows_submit_both_index_endpoints() {
    use crate::git::{Divergence, FileState, FileStatus, Head, RepositoryStatus};

    let root = temporary("rename-action-correspondence");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(repository.clone()));
    app.git.apply_status(RepositoryStatus {
        head: Head::Branch("main".to_owned()),
        upstream: None,
        divergence: Divergence::default(),
        files: vec![FileStatus {
            path: PathBuf::from("after.rs"),
            original_path: Some(PathBuf::from("before.rs")),
            index: FileState::Renamed,
            worktree: FileState::Modified,
        }],
    });
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    assert!(matches!(
        operations
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        GitOperation::Discover { .. }
    ));
    app.open_git_status();
    assert!(matches!(
        operations
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        GitOperation::Refresh { .. }
    ));
    let status = app.active_buffer().to_string();
    let row = status
        .lines()
        .position(|line| line.starts_with("  M "))
        .expect("mixed rename did not have an unstaged destination row");
    let offset = app.active_buffer().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));

    app.discard_git_changes();
    assert_eq!(
        app.git_discard_confirmation.take().unwrap().paths,
        vec![PathBuf::from("before.rs"), PathBuf::from("after.rs")]
    );

    app.stage_files(false);

    let GitOperation::Mutate {
        mutation: GitMutation::Unstage(paths),
        ..
    } = operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap()
    else {
        panic!("rename did not submit an unstage mutation")
    };
    assert_eq!(paths, vec![root.join("before.rs"), root.join("after.rs")]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_copy_row_does_not_unstage_its_independently_changed_source() {
    let root = temporary("copy-action-correspondence");
    fs::create_dir_all(&root).unwrap();
    let git = |arguments: &[&str]| {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Runyte Test"]);
    git(&["config", "user.email", "runyte@example.invalid"]);
    git(&["config", "status.renames", "copies"]);
    fs::write(root.join("source.rs"), "one\ntwo\nthree\n").unwrap();
    git(&["add", "source.rs"]);
    git(&["commit", "-qm", "base"]);
    fs::copy(root.join("source.rs"), root.join("copy.rs")).unwrap();
    fs::write(root.join("source.rs"), "one\ntwo\nthree\nsource changed\n").unwrap();
    fs::write(root.join("copy.rs"), "one\ntwo\nthree\ncopy changed\n").unwrap();
    git(&["add", "source.rs", "copy.rs"]);

    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(crate::git::GitCliProvider::new("git")));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-status").unwrap();
    let buffer = app.active().buffer;
    let row = app.buffers[buffer]
        .to_string()
        .lines()
        .position(|line| line.contains("source.rs → copy.rs"))
        .expect("Git did not report the configured copy row");
    let offset = app.buffers[buffer].line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));

    app.execute_command("git-unstage").unwrap();

    let staged = git(&["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "source.rs\n");
    assert_eq!(
        fs::read_to_string(root.join("source.rs")).unwrap(),
        "one\ntwo\nthree\nsource changed\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn active_rename_actions_submit_both_index_endpoints() {
    use crate::git::{Divergence, FileState, FileStatus, Head, RepositoryStatus};

    let root = temporary("active-rename-action-correspondence");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let after = root.join("after.rs");
    fs::write(&after, "renamed\n").unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(repository.clone()));
    app.git.apply_status(RepositoryStatus {
        head: Head::Branch("main".to_owned()),
        upstream: None,
        divergence: Divergence::default(),
        files: vec![FileStatus {
            path: PathBuf::from("after.rs"),
            original_path: Some(PathBuf::from("before.rs")),
            index: FileState::Renamed,
            worktree: FileState::Unmodified,
        }],
    });
    app.open_file(after.clone()).unwrap();
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    assert!(matches!(
        operations
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        GitOperation::Discover { .. }
    ));

    app.stage_files(false);

    let GitOperation::Mutate {
        mutation: GitMutation::Unstage(paths),
        ..
    } = operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap()
    else {
        panic!("active rename did not submit an unstage mutation")
    };
    assert_eq!(paths, vec![root.join("before.rs"), after]);

    app.discard_git_changes();
    assert_eq!(
        app.git_discard_confirmation.unwrap().paths,
        vec![PathBuf::from("before.rs"), PathBuf::from("after.rs")]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn synchronous_active_rename_staging_refreshes_both_endpoint_bases() {
    use crate::git::{
        Divergence, FileState, FileStatus, Head, MemoryGitProvider, RepositoryStatus,
    };

    let root = temporary("active-rename-cache-convergence");
    fs::create_dir_all(&root).unwrap();
    let before = root.join("before.rs");
    let after = root.join("after.rs");
    fs::write(&before, "before\n").unwrap();
    fs::write(&after, "after\n").unwrap();
    let repository = Repository::new(&root);
    let provider = MemoryGitProvider::new(repository)
        .with_status(RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: vec![FileStatus {
                path: PathBuf::from("after.rs"),
                original_path: Some(PathBuf::from("before.rs")),
                index: FileState::Unmodified,
                worktree: FileState::Renamed,
            }],
        })
        .with_staged("before.rs", "before\n")
        .with_working("after.rs", "after\n");
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(provider));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(before.clone()).unwrap();
    assert!(app.git.tracks(&before));
    app.open_file(after.clone()).unwrap();
    assert!(!app.git.tracks(&after));

    app.stage_files(true);

    assert!(
        !app.git.tracks(&before),
        "the removed source kept its pre-stage index base"
    );
    assert!(app.git.tracks(&after));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failed_mutation_snapshot_schedules_immediate_reconciliation() {
    let root = temporary("mutation-snapshot-retry");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    assert!(matches!(
        operations
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        GitOperation::Discover { .. }
    ));
    app.git.attach(Some(repository.clone()));

    app.apply_git_response(
        GitOperation::Mutate {
            repository: repository.clone(),
            mutation: GitMutation::Stage(vec![root.join("source.rs")]),
            refresh: RefreshSpec::default(),
        },
        GitResponse::Mutation {
            mutation: GitMutation::Stage(vec![root.join("source.rs")]),
            applied_paths: vec![root.join("source.rs")],
            summary: None,
            failure: None,
            snapshot: Box::new(Err(crate::git::GitError::Failed {
                command: "git status".to_owned(),
                code: Some(1),
                signal: None,
                stderr: "transient refresh failure".to_owned(),
            })),
        },
        (None, GitServiceState::Completed),
        RequestedGitViews::default(),
        None,
        None,
    );

    assert!(app.git_state.snapshot_stale());
    assert!(matches!(
        operations
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        GitOperation::Refresh {
            repository: retried,
            ..
        } if retried == repository
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_open_waits_for_the_refreshed_index() {
    use crate::git::{Divergence, FileState, FileStatus, Head, RepositoryStatus, StatusStats};

    let root = temporary("commit-open-refresh");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(repository.clone()));
    app.git.apply_status(RepositoryStatus {
        head: Head::Branch("main".to_owned()),
        upstream: None,
        divergence: Divergence::default(),
        files: Vec::new(),
    });
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();

    app.open_commit_message();

    let operation = operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(matches!(operation, GitOperation::Refresh { .. }));
    assert!(!app.active_buffer().is_commit_message());
    assert!(app.status.contains("checking what is staged"));

    let stale_status = RepositoryStatus {
        head: Head::Branch("main".to_owned()),
        upstream: None,
        divergence: Divergence::default(),
        files: Vec::new(),
    };
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(2),
        operation,
        result: Box::new(Ok(GitResponse::Snapshot(Box::new(RepositorySnapshot {
            repository: repository.clone(),
            generation: RepositoryGeneration::default(),
            started_at: Instant::now(),
            requested: RefreshSpec::default(),
            status: stale_status,
            stats: StatusStats::default(),
            head_oid: Some("a".repeat(40)),
            staged: Vec::new(),
            branches: None,
            staged_diff: None,
            file_diffs: Vec::new(),
            worktrees: None,
            log: None,
            requested_log_anchors: Vec::new(),
            reachable_log_anchors: Vec::new(),
            stashes: None,
        })))),
        state: GitServiceState::Completed,
        coalesced: true,
    });

    assert!(!app.active_buffer().is_commit_message());
    let retry = operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("coalesced refresh did not schedule a fresh index read");
    assert!(matches!(retry, GitOperation::Refresh { .. }));

    let status = RepositoryStatus {
        head: Head::Branch("main".to_owned()),
        upstream: None,
        divergence: Divergence::default(),
        files: vec![FileStatus {
            path: PathBuf::from("external.rs"),
            original_path: None,
            index: FileState::Added,
            worktree: FileState::Unmodified,
        }],
    };
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(3),
        operation: retry,
        result: Box::new(Ok(GitResponse::Snapshot(Box::new(RepositorySnapshot {
            repository,
            generation: RepositoryGeneration::default(),
            started_at: Instant::now(),
            requested: RefreshSpec::default(),
            status,
            stats: StatusStats::default(),
            head_oid: Some("a".repeat(40)),
            staged: Vec::new(),
            branches: None,
            staged_diff: None,
            file_diffs: Vec::new(),
            worktrees: None,
            log: None,
            requested_log_anchors: Vec::new(),
            reachable_log_anchors: Vec::new(),
            stashes: None,
        })))),
        state: GitServiceState::Completed,
        coalesced: false,
    });

    assert!(app.active_buffer().is_commit_message());
    assert!(app.active_buffer().to_string().contains("A external.rs"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelling_a_coalesced_commit_check_does_not_reopen_the_intent() {
    use crate::git::{Divergence, FileState, FileStatus, Head, RepositoryStatus, StatusStats};

    let root = temporary("commit-open-cancelled-coalesced");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(repository));
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    app.open_commit_message();
    let operation = operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(2),
        operation,
        result: Box::new(Err(crate::git::GitError::Failed {
            command: "refresh Git".to_owned(),
            code: None,
            signal: None,
            stderr: "cancelled; the read result was discarded".to_owned(),
        })),
        state: GitServiceState::Cancelled,
        coalesced: true,
    });

    assert!(!app.active_buffer().is_commit_message());
    let reconciliation = operations
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(matches!(reconciliation, GitOperation::Refresh { .. }));
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(3),
        operation: reconciliation,
        result: Box::new(Ok(GitResponse::Snapshot(Box::new(RepositorySnapshot {
            repository: Repository::new(&root),
            generation: RepositoryGeneration::default(),
            started_at: Instant::now(),
            requested: RefreshSpec::default(),
            status: RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: vec![FileStatus {
                    path: PathBuf::from("staged.rs"),
                    original_path: None,
                    index: FileState::Added,
                    worktree: FileState::Unmodified,
                }],
            },
            stats: StatusStats::default(),
            head_oid: Some("a".repeat(40)),
            staged: Vec::new(),
            branches: None,
            staged_diff: None,
            file_diffs: Vec::new(),
            worktrees: None,
            log: None,
            requested_log_anchors: Vec::new(),
            reachable_log_anchors: Vec::new(),
            stashes: None,
        })))),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(
        !app.active_buffer().is_commit_message(),
        "ambient reconciliation revived the cancelled commit intent"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn background_index_refresh_does_not_reopen_a_closed_projection() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_git_index_result("diff --git a/a b/a\n".to_owned());
    let index = app.git_state.index_buffer().unwrap();
    app.closed_buffers.insert(index);
    let buffer_count = app.buffers.len();

    app.update_git_index_result("refreshed\n".to_owned(), false);

    assert_eq!(app.buffers.len(), buffer_count);
    assert!(app.closed_buffers.contains(&index));
}

#[test]
fn stash_actions_refuse_unsaved_buffers_and_every_mutation_is_confirmed() {
    let root = temporary("stash-confirmations");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "base\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(Repository::new(&root)));
    app.open_file(path).unwrap();
    let source = app.active().buffer;
    app.buffers[source].apply(&Transaction::insert(0, "unsaved "));

    app.execute_command("git-stash-tracked named").unwrap();
    assert!(app.git_stash_confirmation.is_none());
    assert!(app.status_error && app.status.contains("unsaved"));

    app.buffers[source].discard_changes_to("base\n").unwrap();
    app.execute_command("git-stash-all recheck").unwrap();
    app.buffers[source].apply(&Transaction::insert(0, "late edit "));
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();
    assert!(app.status_error && app.status.contains("cannot create a stash"));
    app.buffers[source].discard_changes_to("base\n").unwrap();
    for (command, expected) in [
        ("git-stash-tracked worktree", StashScope::TrackedWorktree),
        ("git-stash-all tracked", StashScope::TrackedWorktreeAndIndex),
        (
            "git-stash-untracked everything",
            StashScope::TrackedAndUntracked,
        ),
    ] {
        app.execute_command(command).unwrap();
        assert!(matches!(
            app.git_stash_confirmation,
            Some(GitStashConfirmation {
                mutation: StashMutation::Create { scope, .. },
                ..
            }) if scope == expected
        ));
        assert!(app.status.contains("Enter confirms"));
        let overlay = confirmation_snapshot(&app);
        assert_eq!(overlay.title, "Create stash");
        assert_eq!(overlay.actions[0].label, "create stash");
        assert!(overlay.message.as_deref().is_some_and(|message| {
            message.contains(command.split_whitespace().last().unwrap())
        }));
        app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
            .unwrap();
        assert!(app.git_stash_confirmation.is_none());
    }

    app.open_git_stashes_result(
        vec![StashEntry {
            oid: "a".repeat(40),
            selector: "stash@{0}".to_owned(),
            subject: "kept recovery".to_owned(),
        }],
        true,
    );
    app.execute_command("git-stash-apply").unwrap();
    assert!(matches!(
        app.git_stash_confirmation,
        Some(GitStashConfirmation {
            mutation: StashMutation::Apply { .. },
            ..
        })
    ));
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Apply stash");
    assert_eq!(overlay.actions[0].label, "apply stash");
    assert!(overlay.message.unwrap().contains("stash@{0}"));
    app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();
    app.execute_command("git-stash-drop").unwrap();
    assert!(matches!(
        app.git_stash_confirmation,
        Some(GitStashConfirmation {
            mutation: StashMutation::Drop { .. },
            ..
        })
    ));
    assert!(app.status.contains("Enter confirms"));
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Drop stash");
    assert_eq!(overlay.actions[0].label, "drop stash");
    let message = overlay.message.unwrap();
    assert!(message.contains("stash@{0}"), "{message}");
    assert!(message.contains("kept recovery"), "{message}");
    fs::remove_dir_all(root).unwrap();
}

/// Both stash actions are palette-reachable from any buffer, so their
/// refusal is the one place someone learns where the list is. It names
/// both routes, and takes the key from the registry so that moving the
/// binding fails here instead of leaving a stale key in a message.
#[test]
fn stash_actions_outside_the_list_name_the_way_to_open_it() {
    let root = temporary("stash-refusal-route");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "base\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(Repository::new(&root)));
    app.open_file(path).unwrap();

    let opening = app
        .keymap()
        .global_sequence_for(Mode::Normal, BindingTarget::Colon(ColonCommand::GitStashes))
        .expect("a global binding opens the stash list")
        .to_string();
    assert_eq!(opening, "Space g t");

    for command in ["git-stash-apply", "git-stash-drop"] {
        app.execute_command(command).unwrap();
        assert!(app.status_error, "{command}: {}", app.status);
        assert!(
            app.status.contains(":git-stashes"),
            "{command}: {}",
            app.status
        );
        assert!(app.status.contains(&opening), "{command}: {}", app.status);
        assert!(app.git_stash_confirmation.is_none());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stash_refresh_preserves_object_selection_and_cannot_reopen_a_closed_view() {
    let mut app = App::new(Config::default(), None).unwrap();
    let entry = |byte: char, selector: &str| StashEntry {
        oid: byte.to_string().repeat(40),
        selector: selector.to_owned(),
        subject: format!("stash {byte}"),
    };
    app.open_git_stashes_result(vec![entry('a', "stash@{0}"), entry('b', "stash@{1}")], true);
    let stash = app.active().buffer;
    let second = app.buffers[stash].line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(second));
    app.open_git_stashes_result(
        vec![
            entry('c', "stash@{0}"),
            entry('a', "stash@{1}"),
            entry('b', "stash@{2}"),
        ],
        false,
    );
    let row = app.buffers[stash].offset_to_row(app.active().head());
    assert_eq!(app.git_state.stash_rows()[row].oid, "b".repeat(40));

    app.active_mut().retarget(0);
    app.closed_buffers.insert(stash);
    let count = app.buffers.len();
    app.open_git_stashes_result(vec![entry('d', "stash@{0}")], false);
    assert_eq!(app.buffers.len(), count);
    assert!(
        !app.buffers.iter().enumerate().any(|(index, buffer)| {
            !app.closed_buffers.contains(&index) && buffer.is_git_stash()
        })
    );
}

#[test]
fn background_stash_refresh_preserves_terminal_insert_mode() {
    let mut app = App::new(Config::default(), None).unwrap();
    let entry = |byte: char, selector: &str| StashEntry {
        oid: byte.to_string().repeat(40),
        selector: selector.to_owned(),
        subject: format!("stash {byte}"),
    };
    app.open_git_stashes_result(vec![entry('a', "stash@{0}")], true);
    app.open_terminal(Some("/bin/cat".to_owned()));
    assert!(app.active_terminal().is_some());
    assert_eq!(app.mode, Mode::Insert);

    // An automatic snapshot may have started while the stash projection was
    // visible, then finish after a terminal has covered that same pane. The
    // projection still refreshes behind the terminal, but it must not take
    // terminal input away from the child.
    app.handle_key(KeyStroke::char('x')).unwrap();
    app.open_git_stashes_result(
        vec![entry('b', "stash@{0}"), entry('a', "stash@{1}")],
        false,
    );

    assert_eq!(app.mode, Mode::Insert);
    assert!(app.active_terminal().is_some());
}

#[test]
fn selected_line_staging_refuses_a_dirty_live_buffer_before_submission() {
    let root = temporary("dirty-selected-line-stage");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "base\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(Repository::new(&root)));
    let (service, _events) = crate::git::GitService::spawn(crate::git::GitCliProvider::new("git"));
    app.attach_git_service(service);
    app.open_file(path).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved "));

    app.execute_command("git-stage-lines").unwrap();

    assert!(app.status_error && app.status.contains("save first"));
    assert!(!app.git_state.partial_guards().contains_key(&buffer));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_line_guard_is_stale_after_keyboard_or_pointer_intent() {
    let mut app = App::new(Config::default(), None).unwrap();
    let buffer = app.active().buffer;
    let keyboard = BufferRevisionGuard::new();
    app.git_state
        .partial_guards_mut()
        .insert(buffer, vec![keyboard.clone()]);
    app.handle_key(KeyStroke::new(KeyCode::Char('l'), Modifiers::NONE))
        .unwrap();
    assert!(!keyboard.is_valid());

    let pointer = BufferRevisionGuard::new();
    app.git_state
        .partial_guards_mut()
        .insert(buffer, vec![pointer.clone()]);
    let prepared = app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 20,
            height: 5,
            ..Rect::default()
        },
        editor: Rect {
            width: 20,
            height: 4,
            ..Rect::default()
        },
        status: Rect {
            y: 4,
            width: 20,
            height: 1,
            ..Rect::default()
        },
        message: Rect::default(),
    });
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: 1,
            row: 1,
            modifiers: Modifiers::NONE,
        },
        &prepared,
    )
    .unwrap();
    assert!(!pointer.is_valid());
}

#[test]
fn hunk_staging_refuses_when_the_source_buffer_has_unsaved_text() {
    let root = temporary("dirty-hunk-stage");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "new\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(Repository::new(&root)));
    let (service, _events) = crate::git::GitService::spawn(crate::git::GitCliProvider::new("git"));
    app.attach_git_service(service);
    app.open_file(path.clone()).unwrap();
    let source = app.active().buffer;
    app.buffers[source].apply(&Transaction::insert(0, "unsaved "));
    app.open_git_diff_result(
            DiffScope::Unstaged,
            Some(path),
            "diff --git a/source.txt b/source.txt\n--- a/source.txt\n+++ b/source.txt\n@@ -1 +1 @@\n-old\n+new\n"
                .to_owned(),
        );
    let diff = app.active().buffer;
    let row = (0..app.buffers[diff].len_lines())
        .find(|row| app.buffers[diff].line_string(*row).starts_with("@@ "))
        .unwrap();
    let offset = app.buffers[diff].line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));

    app.execute_command("git-stage-hunk").unwrap();

    assert!(app.status_error && app.status.contains("save first"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hunk_staging_surfaces_unsupported_patch_metadata() {
    let root = temporary("unsupported-hunk-metadata");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "new\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(Repository::new(&root)));
    let (service, _events) = crate::git::GitService::spawn(crate::git::GitCliProvider::new("git"));
    app.attach_git_service(service);
    app.open_git_diff_result(
            DiffScope::Unstaged,
            Some(path),
            "diff --git a/source.txt b/source.txt\nold mode 100644\nnew mode 100755\n--- a/source.txt\n+++ b/source.txt\n@@ -1 +1 @@\n-old\n+new\n"
                .to_owned(),
        );
    let diff = app.active().buffer;
    let row = (0..app.buffers[diff].len_lines())
        .find(|row| app.buffers[diff].line_string(*row).starts_with("@@ "))
        .unwrap();
    let offset = app.buffers[diff].line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));

    app.execute_command("git-stage-hunk").unwrap();

    assert!(app.status_error);
    assert!(app.status.contains("file mode, type, rename, and copy"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepared_hunk_is_discarded_if_its_source_became_dirty_while_git_worked() {
    let root = temporary("late-dirty-hunk-stage");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "saved\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let repository = Repository::new(&root);
    app.git.attach(Some(repository.clone()));
    app.open_file(path.clone()).unwrap();
    let source = app.active().buffer;
    app.buffers[source].apply(&Transaction::insert(0, "late "));
    app.apply_git_response(
        GitOperation::PreparePartial {
            repository,
            selection: Box::new(PartialStageSelection {
                path: path.clone(),
                scope: DiffScope::Unstaged,
                buffer: None,
                guard: None,
                hunk: Some("a".repeat(64)),
                lines: None,
            }),
        },
        GitResponse::PreparedPartial(Box::new(crate::git::PartialStageRequest {
            repository: root.join(".git"),
            fingerprint: crate::git::RepositoryFingerprint {
                head: None,
                index: "0".repeat(64),
            },
            path,
            disk_sha256: "0".repeat(64),
            buffer: None,
            guard: None,
            scope: DiffScope::Unstaged,
            hunk: "a".repeat(64),
            patch: Vec::new(),
        })),
        (None, GitServiceState::Completed),
        RequestedGitViews::default(),
        None,
        None,
    );
    assert!(app.status_error && app.status.contains("stale partial-stage"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_stash_apply_reloads_clean_buffers_to_the_conflict_state() {
    let root = temporary("stash-conflict-reload");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "before\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(Repository::new(&root)));
    app.open_file(path.clone()).unwrap();
    fs::write(&path, "<<<<<<< current\nconflict\n>>>>>>> stash\n").unwrap();
    app.apply_git_mutation_result(
        GitMutation::Stash(StashMutation::Apply {
            oid: "a".repeat(40),
        }),
        Vec::new(),
        None,
        Some(crate::git::GitError::Failed {
            command: "git stash apply".to_owned(),
            code: Some(1),
            signal: None,
            stderr: "stash retained".to_owned(),
        }),
        GitServiceState::Failed,
        None,
    );
    assert!(app.active_buffer().to_string().contains("<<<<<<< current"));
    assert!(!app.active_buffer().dirty);
    fs::remove_dir_all(root).unwrap();
}

/// The path production takes: the drift comes back as a mutation failure
/// from the Git service, and it has to become the offer rather than an
/// error there too.
#[test]
fn a_diverged_pull_from_the_git_service_opens_the_offer_rather_than_an_error() {
    let root = temporary("diverged-service-result");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(Repository::new(&root)));
    let diverged = || crate::git::GitError::Diverged {
        branch: "main".to_owned(),
        upstream: "origin/main".to_owned(),
        ahead: 3,
        behind: 1,
    };

    app.apply_git_mutation_result(
        GitMutation::Pull,
        Vec::new(),
        None,
        Some(diverged()),
        GitServiceState::Failed,
        None,
    );

    assert!(
        !app.status_error,
        "an offer is not an error: {}",
        app.status
    );
    assert!(app.git_pull_rebase.is_some());
    assert!(
        app.status.contains("replay 3 local commits"),
        "{}",
        app.status
    );

    // An uncertain outcome is not an offer: a cancelled or partly applied
    // pull leaves state Runyte cannot describe, so there is nothing safe to
    // propose replaying on top of.
    app.git_pull_rebase = None;
    app.apply_git_mutation_result(
        GitMutation::Pull,
        Vec::new(),
        None,
        Some(diverged()),
        GitServiceState::CompletedWithUncertainState,
        None,
    );

    assert!(app.status_error, "{}", app.status);
    assert!(app.git_pull_rebase.is_none());

    fs::remove_dir_all(root).unwrap();
}
