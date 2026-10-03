// SPDX-License-Identifier: MPL-2.0

#[test]
fn session_table_escapes_control_characters_without_changing_workspace_identity() {
    let project_root = std::path::PathBuf::from("/work/notes\nnext\tcolumn\r\x1b[2J");
    let row = super::WorkspaceRow {
        publication_key: None,
        unread_terminals: None,
        terminal_bell: None,
        id: runyte::workspace::workspace_id(&project_root),
        name: Some("日本\tproject".to_owned()),
        number: None,
        last_active_unix_seconds: None,
        project_root: project_root.clone(),
        running: false,
        incompatible_protocol: None,
        unsaved_buffers: None,
        open_buffers: None,
        pending_wait_requests: None,
        plugin_jobs: None,
        activity_leases: None,
        activities: Vec::new(),
        live_terminals: None,
        terminal_sessions: None,
        terminal_line_activity_unix_seconds: None,
        interactive_attached: None,
        git: None,
        missing_directory: false,
    };
    let table = super::format_session_table([&row]);
    assert_eq!(table.lines().count(), 3, "{table:?}");
    assert!(!table.chars().any(|ch| ch.is_control() && ch != '\n'));
    assert!(table.contains("日本\\tproject"));
    assert!(table.contains("/work/notes\\nnext\\tcolumn\\r\\u{1b}[2J"));
    assert_eq!(row.project_root, project_root);
    assert_eq!(row.id, runyte::workspace::workspace_id(&project_root));
}
