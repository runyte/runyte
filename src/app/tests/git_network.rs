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
    app.ports.git = Some(Box::new(GitCliProvider::new("git")));
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
    let (_fixture, mut app) = fixture();
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
