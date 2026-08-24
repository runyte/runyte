// SPDX-License-Identifier: MPL-2.0

#[test]
fn normal_editor_snapshot_has_no_frontend_or_raw_service_types() {
    let source = include_str!("../src/snapshot.rs");
    for forbidden in ["ratatui", "crossterm", "tree_house", "lsp_types"] {
        assert!(
            !source.contains(forbidden),
            "snapshot boundary names forbidden dependency {forbidden}"
        );
    }
}

#[test]
fn normal_pane_and_status_rendering_do_not_reach_back_into_app_state() {
    let source = include_str!("../src/ui.rs");
    let start = source.find("fn draw_pane(").unwrap();
    let end = source.find("fn draw_picker(").unwrap();
    let normal_surface = &source[start..end];

    for forbidden in [
        "app.buffers",
        "app.panes",
        "app.diagnostics",
        "app.status",
        "app.command",
        "app.jump",
    ] {
        assert!(
            !normal_surface.contains(forbidden),
            "normal renderer reads {forbidden} instead of its snapshot"
        );
    }
}
