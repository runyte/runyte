// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::git::{GitCliProvider, GitProvider, RevisionFileView};

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
        ports.replace_git(Box::new(GitCliProvider::new("git")));
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
    let comparison = GitCliProvider::new("git")
        .compare_revisions(repository, target)
        .unwrap();
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
    let view = GitCliProvider::new("git")
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
