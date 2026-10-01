// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{FrameGeometry, HostPorts},
    clipboard::SystemClipboard,
    command::parse_colon_command,
    git::{CommitDetail, CommitSummary, GitCliProvider, Repository},
    input::{KeyCode, KeyStroke, Modifiers},
    layout::Rect,
    snapshot::{SnapshotRow, TextRole, TextRunKind},
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        Ok(String::new())
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn git_command(root: &std::path::Path, arguments: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .current_dir(root)
        .env("XDG_CONFIG_HOME", root.join(".fixture-config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:05:00+02:00")
        .env("GIT_COMMITTER_DATE", "2026-01-02T12:00:00-05:00")
        .env("GIT_CONFIG_GLOBAL", root.join(".fixture-global"))
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT");
    command
}
fn fixture() -> (Fixture, App) {
    let root = std::env::temp_dir().join(format!(
        "runyte-network-app-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(root.join(".fixture-hooks")).unwrap();
    fs::write(root.join(".fixture-global"), "").unwrap();
    for arguments in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.name", "Graph Author"],
        vec!["config", "user.email", "graph@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "core.hooksPath", ".fixture-hooks"],
        vec!["config", "core.fsmonitor", "false"],
        vec!["commit", "--allow-empty", "-qm", "first commit"],
        vec!["branch", "side"],
    ] {
        assert!(git_command(&root, &arguments).status().unwrap().success());
    }
    let mut app =
        App::new_in_isolated_project(&root, HostPorts::isolated(Box::new(Clipboard))).unwrap();
    app.ports.git = Some(Box::new(
        GitCliProvider::discover(std::env::var_os("PATH").as_deref())
            .expect("these tests need a native Git executable on PATH"),
    ));
    app.git
        .attach(Some(Repository::new(root.canonicalize().unwrap())));
    (Fixture(root), app)
}
fn key(app: &mut App, code: KeyCode, modifiers: Modifiers) {
    app.handle_key(KeyStroke::new(code, modifiers)).unwrap();
}
fn command(app: &mut App, input: &str) {
    app.execute(parse_colon_command(input).unwrap()).unwrap();
}

#[test]
fn commands_tab_scopes_ascii_and_commit_detail_return_use_native_navigation() {
    let (_fixture, mut app) = fixture();
    for c in [' ', 'g', 'n'] {
        key(&mut app, KeyCode::Char(c), Modifiers::NONE);
    }
    assert!(app.active_buffer().is_git_network());
    assert_eq!(
        app.key_binding_scope(),
        crate::keymap::BindingScope::GitNetwork
    );
    let buffer = app.active().buffer;
    let oid = app.selected_network_oid().unwrap();
    app.active_mut().scroll_col = 9;
    let position = super::super::view_position::ViewPosition::capture(app.active());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.active_buffer().is_git_commit_oid(&oid));
    assert_eq!(
        app.active_buffer().line_string(2).trim_end(),
        "Author-time: 2026-01-01 00:05"
    );
    let detail = app.active().buffer;
    app.close_buffer(detail);
    assert_eq!(app.active().buffer, buffer);
    assert_eq!(app.active().selection, position.selection);
    assert_eq!(app.active().scroll_col, 9);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.context_action_menu.is_some());
    key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
    assert!(app.network.ascii);
    assert!(app.active_buffer().line_string(1).contains('*'));
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('h'), Modifiers::NONE);
    assert_eq!(app.network.scope, NetworkScope::Head);
    command(&mut app, "git-network-all");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('r'), Modifiers::NONE);
    assert_eq!(app.list_actions.len(), 2);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(matches!(app.network.scope, NetworkScope::Ref(_)));
    for input in [
        "git-network",
        "git-network-head",
        "git-network-all",
        "git-network-ref refs/heads/side",
        "git-network-choose-ref",
        "next-git-network-page",
        "previous-git-network-page",
        "toggle-git-network-ascii",
    ] {
        command(&mut app, input);
        assert!(app.active_buffer().is_git_network());
        app.list = None;
    }
    assert!(!app.active_buffer().soft_wrap_viable());
    assert!(app.active_buffer().is_read_only());
}

#[test]
fn stale_root_events_preserve_rows_and_explicit_git_refresh_captures_new_generation() {
    let (fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let previous = app.network.pages[0].rows.clone();
    let oid = app.selected_network_oid();
    assert!(
        git_command(&fixture.0, &["commit", "--allow-empty", "-qm", "new tip"])
            .status()
            .unwrap()
            .success()
    );
    app.check_network_roots();
    assert!(app.network.pages[0].stale);
    assert_eq!(app.network.pages[0].rows, previous);
    assert_eq!(app.selected_network_oid(), oid);
    for c in [' ', 'g', 'r'] {
        key(&mut app, KeyCode::Char(c), Modifiers::NONE);
    }
    assert!(!app.network.pages[0].stale);
    assert_eq!(app.network.pages[0].rows.len(), 2);
    assert_eq!(app.selected_network_oid(), oid);
    command(&mut app, "git-network-ref refs/heads/missing");
    assert_eq!(app.network.scope, NetworkScope::All);
    assert_eq!(app.network.pages[0].rows.len(), 2);
}

#[test]
fn graph_snapshot_roles_and_private_protocol_round_trip_preserve_owned_text() {
    let (_fixture, mut app) = merge_fixture();
    command(&mut app, "git-network");
    let prepared = app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 100,
            height: 12,
            ..Default::default()
        },
        editor: Rect {
            width: 100,
            height: 10,
            ..Default::default()
        },
        status: Default::default(),
        message: Default::default(),
    });
    let snapshot = app.snapshot(&prepared);
    let roles = snapshot
        .panes
        .iter()
        .flat_map(|pane| &pane.rows)
        .filter_map(|row| match row {
            SnapshotRow::Text(row) => Some(&row.runs),
            _ => None,
        })
        .flatten()
        .filter_map(|run| match run.kind {
            TextRunKind::Text { role, .. } => Some(role),
            _ => None,
        })
        .collect::<Vec<_>>();
    for role in [TextRole::GitHash, TextRole::GitHead, TextRole::GitLane0] {
        assert!(roles.contains(&role), "missing {role:?}");
    }
    let wire: crate::protocol::EditorSnapshot = snapshot.clone().into();
    let encoded = serde_json::to_vec(&wire).unwrap();
    let decoded: crate::protocol::EditorSnapshot = serde_json::from_slice(&encoded).unwrap();
    let restored: crate::snapshot::EditorSnapshot = decoded.try_into().unwrap();
    assert_eq!(snapshot, restored);
}

#[test]
fn late_response_does_not_consume_new_request_or_steal_focus_after_motion() {
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let page = app.network.pages[0].clone();
    let newer = GitRequestId::from_raw(2);
    app.network.pending = Some(PendingNetwork {
        id: newer,
        page: 0,
        pane: app.active_pane,
        binding: app.active().binding_generation,
        selection: app.active().selection_revision,
        buffer: app.active().buffer,
        scope: NetworkScope::Head,
        anchor: None,
    });
    app.apply_network_response(Some(GitRequestId::from_raw(1)), page.clone());
    assert_eq!(app.network.pending.as_ref().unwrap().id, newer);
    key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
    app.apply_network_response(Some(newer), page);
    assert_eq!(app.network.scope, NetworkScope::All);
    assert!(app.network.pending.is_none());
}

#[test]
fn cached_pages_and_missing_refresh_anchor_restore_safe_page() {
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let first = app.network.pages[0].clone();
    let mut second = first.clone();
    second.rows[0].commit.oid = "f".repeat(40);
    second.rows[0].commit.subject = "second page".into();
    app.network.pages.push(second.clone());
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    assert_eq!(app.network.page, 1);
    key(&mut app, KeyCode::Char('p'), Modifiers::CONTROL);
    assert_eq!(app.network.page, 0);
    assert_eq!(app.network.pages[0], first);
    app.open_network_result(first.clone(), 0, NetworkScope::All, Some("f".repeat(40)));
    assert_eq!(app.network.page, 0);
    assert_eq!(app.network.pages[0], first);
    // A scope switch publishes the first page without searching for the
    // old selected commit excluded by its roots.
    command(&mut app, "git-network-head");
    assert_eq!(app.network.page, 0);
    assert_eq!(app.network.scope, NetworkScope::Head);
    assert!(app.network.loading.is_none());
    let summary: CommitSummary = app.network.pages[0].rows[0].commit.clone();
    app.open_git_commit_detail_result(CommitDetail {
        summary,
        body: "manual detail".into(),
        patch: String::new(),
    });
}

#[test]
fn detail_return_restores_original_cached_graph_page_after_other_navigation() {
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let first = app.network.pages[0].clone();
    let mut second = first.clone();
    second.rows[0].commit.oid = "f".repeat(40);
    second.rows[0].commit.subject = "second page".into();
    app.network.pages.push(second);
    let graph = app.active().buffer;
    let summary = first.rows[0].commit.clone();
    app.active_mut().scroll_col = 7;
    let position = super::super::view_position::ViewPosition::capture(app.active());
    app.open_git_commit_detail_result(CommitDetail {
        summary,
        body: "detail".into(),
        patch: String::new(),
    });
    let detail = app.active().buffer;
    app.network.page = 1;
    app.show_network_page(false);
    app.close_buffer(detail);
    assert_eq!(app.active().buffer, graph);
    assert_eq!(app.network.page, 0);
    assert_eq!(app.active().selection, position.selection);
    assert_eq!(app.active().scroll_col, 7);
    assert_eq!(
        app.selected_network_oid().as_deref(),
        Some(first.rows[0].commit.oid.as_str())
    );
}

#[test]
fn stale_generation_root_check_and_cancelled_read_release_state_without_publication() {
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let id = GitRequestId::from_raw(9);
    let generation = app.network.generation;
    app.network.roots_check = Some((id, generation));
    app.check_network_roots();
    assert_eq!(app.network.roots_check, Some((id, generation)));
    assert!(app.network.roots_dirty);
    app.network_request_failed(id);
    assert!(app.network.roots_check.is_none());
    app.network.generation += 1;
    app.network.roots_check = Some((id, generation));
    app.apply_network_roots(Some(id), NetworkScope::All, Vec::new(), false);
    assert!(!app.network.pages[0].stale);
    assert!(app.network.roots_check.is_none());
    let page = app.network.pages[0].clone();
    app.network.pending = Some(PendingNetwork {
        id,
        page: 0,
        pane: app.active_pane,
        binding: app.active().binding_generation,
        selection: app.active().selection_revision,
        buffer: app.active().buffer,
        scope: NetworkScope::Head,
        anchor: None,
    });
    app.apply_git_service_event(crate::git::GitServiceEvent::Completed {
        id,
        operation: GitOperation::Network {
            repository: app.git.repository().unwrap().clone(),
            request: NetworkRequest {
                scope: NetworkScope::Head,
                cursor: None,
            },
        },
        result: Box::new(Ok(crate::git::GitResponse::Network {
            request: NetworkRequest {
                scope: NetworkScope::Head,
                cursor: None,
            },
            page,
        })),
        state: crate::git::GitServiceState::Cancelled,
        coalesced: false,
    });
    assert!(app.network.pending.is_none());
    assert_eq!(app.network.scope, NetworkScope::All);
}

#[test]
fn atomic_refresh_retains_at_most_two_bounded_generations() {
    use crate::git::network::MAX_NETWORK_PAGES;
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let page = app.network.pages[0].clone();
    app.network.pages = vec![page.clone(); MAX_NETWORK_PAGES];
    app.network.loading = Some(NetworkLoading {
        scope: NetworkScope::All,
        pages: vec![page.clone(); MAX_NETWORK_PAGES - 1],
        anchor: Some("f".repeat(40)),
    });
    assert!(
        app.network.pages.len() + app.network.loading.as_ref().unwrap().pages.len()
            < 2 * MAX_NETWORK_PAGES
    );
    app.open_network_result(page.clone(), MAX_NETWORK_PAGES, NetworkScope::All, None);
    assert_eq!(app.network.pages.len(), MAX_NETWORK_PAGES);
    assert_eq!(
        app.network.loading.as_ref().unwrap().pages.len(),
        MAX_NETWORK_PAGES - 1
    );
    app.open_network_result(page, MAX_NETWORK_PAGES - 1, NetworkScope::All, None);
    assert!(app.network.loading.is_none());
    assert_eq!(app.network.pages.len(), MAX_NETWORK_PAGES);
    assert_eq!(app.network.page, 0);
}

fn merge_fixture() -> (Fixture, App) {
    let (fixture, app) = fixture();
    for args in [
        vec!["commit", "--allow-empty", "-qm", "main change"],
        vec!["checkout", "-q", "side"],
        vec!["commit", "--allow-empty", "-qm", "side change"],
        vec!["checkout", "-q", "main"],
        vec!["merge", "--no-ff", "-qm", "merge branches", "side"],
    ] {
        assert!(git_command(&fixture.0, &args).status().unwrap().success());
    }
    (fixture, app)
}

#[test]
fn connector_rows_are_not_commits_and_refresh_restores_their_block_position() {
    let (_fixture, mut app) = merge_fixture();
    command(&mut app, "git-network");
    let buffer = app.active().buffer;
    assert_eq!(app.network.document_rows[2], Some((0, 1)));
    assert!(commit_line(&app, 1) > 2);
    let offset = app.active_buffer().line_to_offset(2);
    app.active_mut().replace_selection(Selection::point(offset));
    assert!(app.selected_network_oid().is_none());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, buffer);
    command(&mut app, "toggle-git-network-ascii");
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 2);
    assert!(app.active_buffer().line_string(2).contains('\\'));
    command(&mut app, "git-network");
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 2);
    assert!(app.selected_network_oid().is_none());
    let second = commit_line(&app, 1);
    let expected = app.network.pages[0].rows[1].commit.oid.clone();
    let offset = app.active_buffer().line_to_offset(second);
    app.active_mut().replace_selection(Selection::point(offset));
    assert_eq!(app.selected_network_oid().as_ref(), Some(&expected));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.active_buffer().is_git_commit_oid(&expected));
    let detail = app.active().buffer;
    app.close_buffer(detail);
    assert_eq!(
        app.active_buffer().offset_to_row(app.active().head()),
        second
    );
    assert_eq!(app.selected_network_oid().as_ref(), Some(&expected));
}

#[test]
fn log_enter_uses_the_same_author_local_timestamp_as_network_enter() {
    let (_fixture, mut app) = fixture();
    for c in [' ', 'g', 'l'] {
        key(&mut app, KeyCode::Char(c), Modifiers::NONE);
    }
    assert!(app.active_buffer().is_git_log());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.active_buffer().line_string(2).trim_end(),
        "Author-time: 2026-01-01 00:05"
    );
}

#[test]
fn connector_spans_color_the_path_instead_of_the_column() {
    let (_fixture, mut app) = merge_fixture();
    command(&mut app, "git-network");
    let buffer = app.active().buffer;
    let node_color = app.network_role_at(buffer, app.active_buffer().line_to_offset(1) + 20);
    let fork_color = app.network_role_at(buffer, app.active_buffer().line_to_offset(2) + 21);
    assert_eq!(node_color, Some(TextRole::GitLane0));
    assert_eq!(fork_color, Some(TextRole::GitLane1));
    // One column's adjacent cells belong to two different paths.
    assert_eq!(
        app.network_role_at(buffer, app.active_buffer().line_to_offset(2) + 20),
        node_color
    );
    assert_ne!(node_color, fork_color);
    let before = app.network.spans.clone();
    command(&mut app, "toggle-git-network-ascii");
    assert_eq!(
        app.network
            .spans
            .iter()
            .map(|(_, role)| role)
            .collect::<Vec<_>>(),
        before.iter().map(|(_, role)| role).collect::<Vec<_>>()
    );
    assert_eq!(
        app.network_role_at(buffer, app.active_buffer().line_to_offset(2) + 21),
        fork_color
    );
}

#[test]
fn ascii_toggle_preserves_multirange_selections_and_independent_split_positions() {
    use crate::selection::Range;
    let (_fixture, mut app) = merge_fixture();
    command(&mut app, "git-network");
    let first = app.active_pane;
    let start = app.active_buffer().line_to_offset(2);
    let end = app.active_buffer().line_to_offset(commit_line(&app, 1));
    app.active_mut().replace_selection(Selection::new(
        vec![Range::new(start + 20, start + 22), Range::point(end + 2)],
        1,
    ));
    app.active_mut().scroll_col = 7;
    app.split(crate::layout::Axis::Horizontal, None).unwrap();
    let second = app.active_pane;
    assert_ne!(first, second);
    app.active_mut()
        .replace_selection(Selection::point(end + 5));
    app.active_mut().scroll_col = 3;
    let positions = |app: &App| {
        app.panes
            .iter()
            .filter(|(_, pane)| pane.buffer == app.active().buffer)
            .map(|(id, pane)| {
                let coordinates = |offset| {
                    let row = app.active_buffer().offset_to_row(offset);
                    (row, offset - app.active_buffer().line_to_offset(row))
                };
                (
                    *id,
                    pane.selection.primary_index(),
                    pane.selection
                        .ranges()
                        .iter()
                        .map(|range| (coordinates(range.anchor), coordinates(range.head)))
                        .collect::<Vec<_>>(),
                    pane.scroll_row,
                    pane.scroll_col,
                )
            })
            .collect::<Vec<_>>()
    };
    let before = positions(&app);
    command(&mut app, "toggle-git-network-ascii");
    assert_eq!(positions(&app), before);
    command(&mut app, "toggle-git-network-ascii");
    assert_eq!(positions(&app), before);
}

#[test]
fn detail_return_survives_ascii_toggle_in_another_pane() {
    let (_fixture, mut app) = merge_fixture();
    command(&mut app, "git-network");
    let graph_pane = app.active_pane;
    app.split(crate::layout::Axis::Horizontal, None).unwrap();
    let detail_pane = app.active_pane;
    let row = commit_line(&app, 1);
    let offset = app.active_buffer().line_to_offset(row) + 2;
    app.active_mut().replace_selection(Selection::point(offset));
    let oid = app.selected_network_oid().unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let detail = app.active().buffer;
    app.active_pane = graph_pane;
    command(&mut app, "toggle-git-network-ascii");
    app.active_pane = detail_pane;
    app.close_buffer(detail);
    assert_eq!(app.selected_network_oid(), Some(oid));
    assert_eq!(
        app.active().head(),
        app.active_buffer().line_to_offset(row) + 2
    );
}

fn commit_line(app: &App, index: usize) -> usize {
    app.network
        .document_rows
        .iter()
        .position(|identity| *identity == Some((index, 0)))
        .unwrap()
}

fn tall_page(app: &App) -> NetworkPage {
    let mut page = app.network.pages[0].clone();
    let mut lanes = crate::git::network::GraphLanes::default();
    let template = page.rows[0].commit.clone();
    page.rows = (0..100)
        .map(|i| {
            let mut commit = template.clone();
            commit.oid = format!("{i:040x}");
            commit.parents = if i == 99 {
                vec![]
            } else {
                vec![format!("{:040x}", i + 1)]
            };
            lanes.row(commit, false)
        })
        .collect();
    page
}

fn assert_network_caret_visible(app: &mut App) {
    let geometry = FrameGeometry {
        screen: Rect {
            width: 100,
            height: 14,
            ..Default::default()
        },
        editor: Rect {
            width: 100,
            height: 12,
            ..Default::default()
        },
        status: Default::default(),
        message: Default::default(),
    };
    app.prepare_view(geometry);
    let row = app.active_buffer().offset_to_row(app.active().head());
    assert!(row >= app.active().scroll_row);
    assert!(
        row < app.active().scroll_row + 10,
        "caret row {row}, viewport starts at {}",
        app.active().scroll_row
    );
}

#[test]
fn paging_from_a_scrolled_page_reveals_the_new_caret() {
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let page = tall_page(&app);
    app.network.pages = vec![page.clone(), page];
    app.show_network_page(true);
    let offset = app.active_buffer().line_to_offset(85);
    app.active_mut().replace_selection(Selection::point(offset));
    app.active_mut().scroll_row = 80;
    app.active_mut().preserve_scroll = true;
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    assert_eq!(app.active_buffer().offset_to_row(app.active().head()), 1);
    assert!(!app.active().preserve_scroll);
    assert_network_caret_visible(&mut app);
    app.active_mut().scroll_row = 80;
    app.active_mut().preserve_scroll = true;
    key(&mut app, KeyCode::Char('p'), Modifiers::CONTROL);
    assert_network_caret_visible(&mut app);
}

#[test]
fn refresh_reveals_an_anchor_that_moves_more_than_one_screen() {
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let page = tall_page(&app);
    let anchor = page.rows[0].commit.oid.clone();
    app.network.pages = vec![page.clone()];
    app.show_network_page(true);
    let mut refreshed = page;
    refreshed.rows.rotate_left(1);
    app.active_mut().preserve_scroll = true;
    app.open_network_result(refreshed, 0, NetworkScope::All, Some(anchor.clone()));
    assert_eq!(app.selected_network_oid(), Some(anchor));
    assert!(!app.active().preserve_scroll);
    assert_network_caret_visible(&mut app);
}

#[test]
fn ascii_toggle_preserves_the_exact_selected_head_row_subject() {
    let (_fixture, mut app) = fixture();
    command(&mut app, "git-network");
    let text = app.active_buffer().line_string(1);
    let column = text[..text.find("first commit").unwrap()].chars().count();
    let offset = app.active_buffer().line_to_offset(1) + column;
    app.active_mut()
        .replace_selection(Selection::single(crate::selection::Range::new(
            offset,
            offset + 5,
        )));
    for _ in 0..2 {
        command(&mut app, "toggle-git-network-ascii");
        let row = app.active_buffer().line_string(1);
        let actual_column = app.active().head() - app.active_buffer().line_to_offset(1);
        assert_eq!(actual_column, column + 5);
        assert_eq!(
            row.chars().skip(column).take(5).collect::<String>(),
            "first"
        );
        assert_eq!(app.active().selection.primary().anchor, offset);
    }
}
