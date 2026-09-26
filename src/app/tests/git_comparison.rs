// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::git::{GitCliProvider, GitProvider, RevisionFileView};

/// The Git this machine has, resolved to the absolute path Windows requires of
/// a background program.
fn git() -> GitCliProvider {
    GitCliProvider::discover(std::env::var_os("PATH").as_deref())
        .expect("these tests need `git` on PATH")
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = temporary("committed-comparison");
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        #[cfg(windows)]
        let root = crate::windows_fs::ordinary_working_directory(&root).unwrap();
        let fixture = Self(root);
        fixture.git(&["init", "-q", "-b", "main"]);
        fixture.git(&["config", "user.name", "Runyte Test"]);
        fixture.git(&["config", "user.email", "runyte@example.invalid"]);
        fs::write(fixture.0.join("a.txt"), "base\n").unwrap();
        fs::write(fixture.0.join("b.txt"), "old\n").unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "base"]);
        fixture.git(&["checkout", "-qb", "feature"]);
        fs::write(fixture.0.join("a.txt"), "feature\n").unwrap();
        fs::write(fixture.0.join("b.txt"), "new\n").unwrap();
        fixture.git(&["commit", "-qam", "feature"]);
        fixture.git(&["checkout", "-q", "main"]);
        fixture
    }
    fn git(&self, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn app(&self) -> App {
        let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        )))));
        ports.replace_git(Box::new(git()));
        App::new_in_isolated_project(&self.0, ports).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn row(app: &mut App, needle: &str) {
    let row = app
        .active_buffer()
        .to_string()
        .lines()
        .position(|line| line.contains(needle))
        .unwrap();
    let offset = app.active_buffer().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));
}
fn compare(app: &mut App) {
    for ch in [' ', 'g', 'b'] {
        press(app, ch);
    }
    row(app, "feature");
    context_action(app, 'd');
    assert_eq!(app.key_binding_scope(), BindingScope::GitComparison);
}
fn geometry(width: u16) -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            width,
            height: 25,
            ..Rect::default()
        },
        editor: Rect {
            width,
            height: 23,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    }
}

fn assert_count_runs(app: &mut App, width: u16, expected: &[(usize, &str, &str)]) {
    use crate::{
        git::CountKind,
        snapshot::{SnapshotRow, TextRunKind},
    };
    let prepared = app.prepare_view(geometry(width));
    let snapshot = app.snapshot(&prepared);
    let mut actual = std::collections::BTreeMap::<usize, (String, String)>::new();
    for row in &snapshot.pane(app.active_pane).unwrap().rows {
        let SnapshotRow::Text(row) = row else {
            continue;
        };
        for run in &row.runs {
            if let TextRunKind::Text {
                count: Some(kind), ..
            } = run.kind
            {
                let counts = actual.entry(row.document_row).or_default();
                match kind {
                    CountKind::Added => counts.0.push_str(&run.text),
                    CountKind::Removed => counts.1.push_str(&run.text),
                }
            }
        }
    }
    assert_eq!(
        actual,
        expected
            .iter()
            .map(|(row, added, removed)| { (*row, (added.to_string(), removed.to_string())) })
            .collect()
    );
}

#[test]
fn committed_comparison_count_colors_follow_branch_and_worktree_refresh() {
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    let fixture = Fixture::new();
    let linked = fixture.0.join("linked");
    fixture.git(&["worktree", "add", linked.to_str().unwrap(), "feature"]);
    for source in ['b', 'w'] {
        let mut app = fixture.app();
        app.theme.change_added = crate::config::Color::Green;
        app.theme.change_removed = crate::config::Color::Red;
        for ch in [' ', 'g', source] {
            press(&mut app, ch);
        }
        row(&mut app, "feature");
        context_action(&mut app, 'd');
        for width in [120, 35, 120] {
            assert_count_runs(
                &mut app,
                width,
                &[(1, "+2", "-2"), (4, "+1", "-1"), (5, "+1", "-1")],
            );
            let mut terminal = Terminal::new(TestBackend::new(width, 25)).unwrap();
            terminal
                .draw(|frame| {
                    let prepared = app.prepare_view(crate::ui::frame_geometry(frame.area()));
                    let snapshot = app.snapshot(&prepared);
                    crate::ui::render_exact_colors_for_test(
                        frame,
                        &app,
                        &snapshot,
                        &crate::key_hints::KeyHintState::default(),
                    );
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let mut counted_rows = 0;
            for y in 0..25 {
                let line: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
                for counts in ["+2 -2", "+1 -1"] {
                    if let Some(start) = line.find(counts) {
                        let x = line[..start].chars().count() as u16;
                        for offset in [0, 1] {
                            assert_eq!(buffer[(x + offset, y)].fg, Color::Green);
                        }
                        for offset in [3, 4] {
                            assert_eq!(buffer[(x + offset, y)].fg, Color::Red);
                        }
                        assert_ne!(buffer[(x + 2, y)].fg, Color::Green);
                        assert_ne!(buffer[(x + 2, y)].fg, Color::Red);
                        counted_rows += 1;
                    }
                }
            }
            assert_eq!(counted_rows, 3);
        }
        fs::write(linked.join("a.txt"), "feature\nextra\n").unwrap();
        fixture.git(&["-C", linked.to_str().unwrap(), "commit", "-qam", "extra"]);
        app.execute_command("git-refresh").unwrap();
        assert_count_runs(
            &mut app,
            120,
            &[(1, "+3", "-2"), (4, "+2", "-1"), (5, "+1", "-1")],
        );
        fixture.git(&["-C", linked.to_str().unwrap(), "reset", "--hard", "HEAD~1"]);
    }
}

#[test]
fn committed_comparison_count_ranges_exclude_paths_labels_and_metadata() {
    use crate::git::{ComparisonTarget, LineStats, RevisionComparison, RevisionFile};
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let file = |left: Option<&str>, right: Option<&str>, stats| RevisionFile {
        left: left.map(PathBuf::from),
        right: right.map(PathBuf::from),
        left_object: "a".repeat(40),
        right_object: "b".repeat(40),
        left_mode: "100644".into(),
        right_mode: "100644".into(),
        stats,
    };
    let mut comparison = RevisionComparison {
        target: ComparisonTarget::Branch {
            reference: "refs/heads/feature".into(),
            label: "feature".into(),
        },
        left_label: "main+22".into(),
        right_label: "feature-33".into(),
        left_oid: "a".repeat(40),
        right_oid: "b".repeat(40),
        files: vec![
            file(
                Some("界é+34"),
                Some("改e\u{301}-16"),
                Some(LineStats::new(3032, 810)),
            ),
            file(None, Some("added+1"), Some(LineStats::new(34, 0))),
            file(Some("removed-1"), None, Some(LineStats::new(0, 16))),
            file(Some("binary+1"), Some("binary+1"), None),
            file(None, Some("empty+1"), Some(LineStats::default())),
            file(Some("empty-1"), None, Some(LineStats::default())),
            file(Some("old+1"), Some("new-1"), Some(LineStats::default())),
            file(Some("mode+1"), Some("mode+1"), Some(LineStats::default())),
        ],
    };
    comparison.files[7].right_mode = "100755".into();
    let repository = crate::git::Repository::new(&fixture.0);
    app.show_revision_comparison(repository.clone(), comparison.clone());
    for width in [160, 42, 160] {
        assert_count_runs(
            &mut app,
            width,
            &[
                (1, "+3066", "-826"),
                (4, "+3032", "-810"),
                (5, "+34", "-0"),
                (6, "+0", "-16"),
            ],
        );
    }
    comparison.files.clear();
    app.show_revision_comparison(repository, comparison);
    assert_count_runs(&mut app, 120, &[(1, "+0", "-0")]);
    assert!(
        app.active_buffer()
            .to_string()
            .contains("No committed differences.")
    );
}

#[test]
fn committed_comparison_keys_restore_file_and_scroll_from_patch_and_both_split_sides() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    compare(&mut app);
    let list = app.active().buffer;
    assert!(app.active_buffer().is_read_only());
    assert!(app.active_buffer().to_string().contains("main"));
    row(&mut app, "b.txt");
    app.active_mut().scroll_row = 2;
    let selection = app.active().selection.clone();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.key_binding_scope(), BindingScope::GitRevisionDiff);
    assert!(app.active_buffer().to_string().contains("-old\n+new"));
    assert!(
        app.keymap
            .context_actions(app.key_binding_scope())
            .next()
            .is_none()
    );
    app.execute_command("close").unwrap();
    assert_eq!(app.active().buffer, list);
    assert_eq!(app.active().selection, selection);
    assert_eq!(app.active().scroll_row, 2);
    for (side, command) in [
        (Side::Left, "window-close"),
        (Side::Right, "close"),
        (Side::Right, "diff-off"),
    ] {
        context_action(&mut app, 'd');
        let diff = app.diffs.last().unwrap();
        assert_eq!(
            app.buffers[diff.side(Side::Left).buffer].to_string(),
            "old\n"
        );
        assert_eq!(
            app.buffers[diff.side(Side::Right).buffer].to_string(),
            "new\n"
        );
        app.active_pane = diff.side(side).pane;
        app.execute_command(command).unwrap();
        assert!(app.diffs.is_empty());
        assert_eq!(app.panes.len(), 1);
        assert_eq!(app.active().buffer, list);
        assert_eq!(app.active().selection, selection);
        assert_eq!(app.active().scroll_row, 2);
    }
}

#[test]
fn committed_comparison_refresh_resize_and_status_action_keep_semantic_targets() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    compare(&mut app);
    row(&mut app, "b.txt");
    let list = app.active().buffer;
    app.prepare_view(geometry(120));
    assert!(
        app.active_buffer()
            .to_string()
            .lines()
            .nth(3)
            .unwrap()
            .contains("main")
    );
    app.prepare_view(geometry(25));
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 5);
    assert_eq!(
        app.active_buffer().to_string().lines().nth(3),
        Some("File · Changes")
    );
    fixture.git(&["checkout", "-q", "feature"]);
    fs::write(fixture.0.join("b.txt"), "later\n").unwrap();
    fixture.git(&["commit", "-qam", "later"]);
    fixture.git(&["checkout", "-q", "main"]);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.active_buffer().to_string().contains("+new"));
    app.execute_command("close").unwrap();
    app.execute_command("git-refresh").unwrap();
    assert_eq!(app.active().buffer, list);
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 5);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.active_buffer().to_string().contains("+later"));
    app.execute_command("close").unwrap();
    fs::write(fixture.0.join("b.txt"), "local\n").unwrap();
    for ch in [' ', 'g', 'g'] {
        press(&mut app, ch);
    }
    row(&mut app, "b.txt");
    let status = app.active().buffer;
    context_action(&mut app, 'd');
    let diff = app.diffs.last().unwrap();
    assert_eq!(
        app.buffers[diff.side(Side::Left).buffer].to_string(),
        "old\n"
    );
    assert_eq!(
        app.buffers[diff.side(Side::Right).buffer].to_string(),
        "local\n"
    );
    app.execute_command("diff-off").unwrap();
    assert_eq!(app.active().buffer, status);
}

#[test]
fn committed_comparison_async_results_preserve_foreground_ownership() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    app.open_git_branches();
    row(&mut app, "feature");
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.ports.git_service = Some(service);
    context_action(&mut app, 'd');
    let id = GitRequestId::from_raw(1);
    let operation = operations.recv_timeout(Duration::from_secs(2)).unwrap();
    let GitOperation::CompareRevisions { repository, target } = &operation else {
        panic!()
    };
    let comparison = git().compare_revisions(repository, target).unwrap();
    app.open_scratch_buffer();
    let scratch = app.active().buffer;
    app.apply_git_service_event(GitServiceEvent::Completed {
        id,
        operation,
        result: Box::new(Ok(GitResponse::RevisionComparison(comparison))),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert_eq!(app.active().buffer, scratch);

    app.ports.git_service = None;
    compare(&mut app);
    row(&mut app, "b.txt");
    let list = app.active().buffer;
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.ports.git_service = Some(service);
    context_action(&mut app, 'd');
    let id = GitRequestId::from_raw(1);
    let operation = operations.recv_timeout(Duration::from_secs(2)).unwrap();
    let GitOperation::RevisionFile {
        repository,
        comparison,
        file,
        split,
    } = &operation
    else {
        panic!()
    };
    assert!(comparison.files.is_empty());
    let view = git()
        .revision_file(repository, comparison, file, *split)
        .unwrap();
    assert!(matches!(view, RevisionFileView::Split(_)));
    app.apply_git_service_event(GitServiceEvent::Completed {
        id,
        operation,
        result: Box::new(Ok(GitResponse::RevisionFile(view))),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert_eq!(app.panes.len(), 2);
    app.execute_command("diff-off").unwrap();
    assert_eq!(app.active().buffer, list);
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 5);
}

#[test]
fn committed_comparison_worktree_action_uses_the_selected_checkout_tip() {
    let fixture = Fixture::new();
    let linked = fixture.0.join("linked");
    fixture.git(&[
        "worktree",
        "add",
        "--detach",
        linked.to_str().unwrap(),
        "feature",
    ]);
    let mut app = fixture.app();
    for ch in [' ', 'g', 'w'] {
        press(&mut app, ch);
    }
    row(&mut app, "linked");
    context_action(&mut app, 'd');
    assert_eq!(app.key_binding_scope(), BindingScope::GitComparison);
    assert!(app.active_buffer().to_string().contains("detached"));
    assert!(app.active_buffer().to_string().contains("2 files"));
}

#[test]
fn committed_comparison_return_positions_belong_to_each_originating_pane() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    compare(&mut app);
    let first = app.active_pane;
    app.split(Axis::Horizontal, None).unwrap();
    let second = app.active_pane;
    row(&mut app, "b.txt");
    context_action(&mut app, 'd');
    app.activate_pane(first);
    row(&mut app, "a.txt");
    context_action(&mut app, 'd');
    assert_eq!(app.diffs.len(), 2);
    app.activate_pane(second);
    app.execute_command("diff-off").unwrap();
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 5);
    app.activate_pane(first);
    app.execute_command("diff-off").unwrap();
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 4);
    assert_eq!(app.panes.len(), 2);
}

#[test]
fn committed_comparison_reprojects_visible_and_hidden_positions_across_resize_and_refresh() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    compare(&mut app);
    app.prepare_view(geometry(120));
    let first = app.active_pane;
    let list = app.active().buffer;
    row(&mut app, "b.txt");
    app.split(Axis::Horizontal, None).unwrap();
    let second = app.active_pane;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.activate_pane(first);
    app.prepare_view(geometry(40));
    assert_eq!(
        app.buffers[list].to_string().lines().nth(3),
        Some("File · Changes")
    );
    fixture.git(&["checkout", "-q", "feature"]);
    fs::write(fixture.0.join("0.txt"), "earlier row\n").unwrap();
    fixture.git(&["add", "0.txt"]);
    fixture.git(&["commit", "-qm", "insert row"]);
    fixture.git(&["checkout", "-q", "main"]);
    app.execute_command("git-refresh").unwrap();
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 6);
    app.activate_pane(second);
    app.execute_command("close").unwrap();
    assert_eq!(app.active().buffer, list);
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 6);
    assert!(app.active_buffer().line_string(6).contains("b.txt"));
}

#[test]
fn committed_comparison_reuses_inactive_split_buffers() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    compare(&mut app);
    context_action(&mut app, 'd');
    let first = app.diffs.last().unwrap().clone();
    app.execute_command("diff-off").unwrap();
    for _ in 0..4 {
        context_action(&mut app, 'd');
        let next = app.diffs.last().unwrap();
        for side in [Side::Left, Side::Right] {
            assert_eq!(first.side(side).buffer, next.side(side).buffer);
        }
        app.execute_command("diff-off").unwrap();
    }
}
