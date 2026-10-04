// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{file_picker::ScanScope, service_health::PLAIN_SESSION_REASON};

// -- Plain sessions ----------------------------------------------------
//
// A standalone session with no workspace. These cover the editor half: what
// is refused, what search covers, and how a workspace is added later. The
// launch decision is covered beside it in `src/main.rs`.

/// A plain app whose launch directory is `root`, with `file` open.
fn plain_app(root: &Path, file: &Path) -> App {
    let mut app = App::new_in_isolated_project(
        root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enter_plain_session();
    app.open_file(file.to_path_buf()).unwrap();
    app
}

/// `root/etc/nginx/nginx.conf` with a sibling, plus a file directly in
/// `root` that a project-wide search would find and a plain one must not.
fn system_tree(name: &str) -> (PathBuf, PathBuf) {
    let root = temporary(name);
    fs::create_dir_all(root.join("etc/nginx/sites")).unwrap();
    fs::write(root.join("etc/nginx/nginx.conf"), "needle\n").unwrap();
    fs::write(root.join("etc/nginx/sites/default"), "needle\n").unwrap();
    fs::write(root.join("outside.txt"), "needle\n").unwrap();
    let root = root.canonicalize().unwrap();
    let file = root.join("etc/nginx/nginx.conf");
    (root, file)
}

#[test]
fn a_plain_session_reports_workspace_services_as_needing_a_workspace() {
    let (root, file) = system_tree("plain-capabilities");
    let app = plain_app(&root, &file);
    let capabilities = app.command_capabilities();
    for availability in [
        &capabilities.lsp_manager,
        &capabilities.lsp_document,
        &capabilities.git_project,
        &capabilities.git_refresh,
        &capabilities.git_fetch_branch,
        &capabilities.git_conflict,
        &capabilities.persistent_session,
        &capabilities.session_controls,
    ] {
        assert_eq!(availability.reason(), Some(PLAIN_SESSION_REASON));
    }

    let mut workspace = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    workspace.open_file(file).unwrap();
    assert_ne!(
        workspace.command_capabilities().git_project.reason(),
        Some(PLAIN_SESSION_REASON)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_language_server_mcp_plugin_and_session_commands_refuse_in_a_plain_session() {
    let (root, file) = system_tree("plain-refusals");
    let mut app = plain_app(&root, &file);
    for command in [
        "git-status",
        "git-refresh",
        "lsp-status",
        "lsp-trust",
        "format",
        "mcp",
        "plugins",
        "session-list",
        "session-stop",
    ] {
        app.status.clear();
        let outcome = app.execute_command(command).unwrap();
        assert!(
            matches!(outcome, CommandOutcome::Unavailable(_)),
            "{command}: {outcome:?}"
        );
        assert_eq!(app.status, PLAIN_SESSION_REASON, "{command}");
    }

    // A key binding reaches the same refusal as the typed command.
    app.status.clear();
    app.execute_editor_command(EditorCommand::GotoDefinition)
        .unwrap();
    assert_eq!(app.status, PLAIN_SESSION_REASON);
    assert!(!root.join(".runyte").exists());
    assert!(!root.join("etc/nginx/.runyte").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_plain_finder_covers_the_active_directory_and_names_it() {
    let (root, file) = system_tree("plain-finder");
    let mut app = plain_app(&root, &file);

    app.open_project_picker().unwrap();
    let picker = app.picker.as_ref().unwrap();
    let nginx = root.join("etc/nginx");
    assert_eq!(picker.root, nginx);
    assert_eq!(picker.scope, ScanScope::contained(Some(nginx.clone())));
    // The title names the directory even though nothing else would say it:
    // the scope follows the reader rather than a fixed root.
    assert_eq!(
        picker.scope_label(&app.project_root),
        Some(nginx.display().to_string())
    );
    let found = picker
        .views()
        .map(|entry| entry.path.to_path_buf())
        .collect::<Vec<_>>();
    assert!(found.contains(&nginx.join("sites/default")), "{found:?}");
    assert!(!found.contains(&root.join("outside.txt")), "{found:?}");
    app.close_file_picker();

    app.open_all_files_picker().unwrap();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.root, nginx);
    assert_eq!(picker.scope, ScanScope::contained(None));
    assert_eq!(
        picker.scope_label(&app.project_root),
        Some(format!("all files in {}", nginx.display()))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn plain_project_search_is_rooted_at_the_active_directory() {
    let (root, file) = system_tree("plain-search");
    let mut app = plain_app(&root, &file);
    assert_eq!(app.search_root(), root.join("etc/nginx"));

    let nginx = root.join("etc/nginx");

    // The prompt names the directory, because there is no workspace to name.
    app.mode = Mode::Command;
    app.prompt_kind = PromptKind::GlobalSearch(SearchMode::Regex);
    let prepared = app.prepare_view(crate::ui::frame_geometry(ratatui::layout::Rect::new(
        0, 0, 200, 10,
    )));
    let prompt = app.snapshot(&prepared).status.interaction_line;
    assert_eq!(prompt, format!("search {} (regex): ", nginx.display()));
    app.mode = Mode::Normal;
    app.prompt_kind = PromptKind::Command;

    app.open_global_search("needle", SearchMode::Sensitive);
    let text = app.active_buffer().to_string();
    assert!(
        text.contains(&format!("Directory: {}", nginx.display())),
        "{text}"
    );
    assert!(text.contains("sites/default:1:1"), "{text}");
    assert!(!text.contains("outside.txt"), "{text}");
    assert_eq!(
        app.status,
        format!("search in {}: 2 results", nginx.display())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn goto_file_in_a_plain_session_does_not_resolve_against_the_launch_directory() {
    let (root, file) = system_tree("plain-goto-file");
    let mut app = plain_app(&root, &file);
    let directory = Some(root.join("etc/nginx"));
    assert!(
        app.literal_navigation_candidates("outside.txt", directory.clone())
            .is_empty()
    );
    assert_eq!(
        app.literal_navigation_candidates("sites/default", directory.clone()),
        [root.join("etc/nginx/sites/default")]
    );

    app.plain = false;
    assert_eq!(
        app.literal_navigation_candidates("outside.txt", directory),
        [root.join("outside.txt")]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_init_gives_a_plain_session_a_workspace_at_the_active_directory() {
    let (root, file) = system_tree("plain-workspace-init");
    let mut app = plain_app(&root, &file);
    assert!(!app.take_workspace_services_request());

    app.execute_command("workspace-init").unwrap();

    let nginx = root.join("etc/nginx");
    assert!(nginx.join(".runyte").is_dir());
    assert!(!root.join(".runyte").exists());
    assert!(!app.is_plain());
    assert_eq!(app.project_root, nginx);
    assert_eq!(app.state_root, nginx.join(".runyte"));
    assert_eq!(app.working_directory, nginx);
    assert!(app.take_workspace_services_request());
    assert!(!app.take_workspace_services_request(), "taken once");
    assert_ne!(
        app.command_capabilities().git_project.reason(),
        Some(PLAIN_SESSION_REASON)
    );

    // A second initialization has nothing to do.
    app.execute_command("workspace-init").unwrap();
    assert!(
        app.status.contains("already has a workspace"),
        "{}",
        app.status
    );
    assert!(!app.take_workspace_services_request());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_init_accepts_an_explicit_directory() {
    let (root, file) = system_tree("plain-workspace-init-explicit");
    let mut app = plain_app(&root, &file);

    app.execute_command(&format!("workspace-init {}", root.join("etc").display()))
        .unwrap();

    assert!(root.join("etc/.runyte").is_dir());
    assert!(!root.join("etc/nginx/.runyte").exists());
    assert_eq!(app.project_root, root.join("etc"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_init_reports_a_directory_it_cannot_use() {
    let (root, file) = system_tree("plain-workspace-init-missing");
    let mut app = plain_app(&root, &file);

    app.execute_command(&format!(
        "workspace-init {}",
        root.join("missing").display()
    ))
    .unwrap();

    assert!(app.is_plain());
    assert!(!app.take_workspace_services_request());
    assert!(!root.join("missing").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_filesystem_plan_says_when_it_runs_as_root() {
    let root = temporary("plain-root-plan");
    fs::create_dir_all(&root).unwrap();
    let snapshot = crate::fs_plan::DirectorySnapshot::read(&root).unwrap();
    let desired = vec![crate::fs_plan::DesiredEntry::create(
        "new".to_owned(),
        crate::fs_plan::EntryKind::File,
    )];
    let plan = crate::fs_plan::FsPlan::build(root.clone(), snapshot, desired).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.fs_confirmation = Some(FsConfirmation {
        origin: super::FsConfirmationOrigin::Explorer {
            buffer: app.active().buffer,
        },
        plan,
        selected: 0,
    });
    let title = |app: &App| {
        app.overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilesystemConfirmation)
            .expect("filesystem plan overlay")
            .title
    };

    assert!(!title(&app).contains("as root"));
    app.note_running_as_root(true);
    assert!(title(&app).contains("Filesystem plan · as root"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_init_gives_the_host_the_new_workspace_identity() {
    let (root, file) = system_tree("plain-workspace-init-identity");
    let mut host = crate::workspace::WorkspaceHost::new(plain_app(&root, &file));
    assert_eq!(host.identity().root(), root);

    host.app_mut()
        .initialize_workspace(Some(root.join("etc/nginx")));
    host.refresh_workspace_identity();

    // Context registration pairs this identity with the editor's project
    // root, and the two must name the same workspace.
    assert_eq!(host.identity().root(), root.join("etc/nginx"));
    assert_eq!(host.identity().root(), host.app().project_root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_only_and_plain_only_commands_grey_out_in_the_other_session() {
    let (root, file) = system_tree("plain-greying");
    let mut app = plain_app(&root, &file);
    let availability = |app: &App, name: &str| {
        let spec = crate::command::resolve_command(name).unwrap();
        app.command_capabilities().command_availability(spec)
    };
    for name in [
        "mcp",
        "lsp-trust",
        "plugins",
        "plugin-stop",
        "plugin-restart",
    ] {
        assert_eq!(
            availability(&app, name).reason(),
            Some(PLAIN_SESSION_REASON),
            "{name}"
        );
    }
    assert!(availability(&app, "workspace-init").is_available());

    app.execute_command("workspace-init").unwrap();
    for name in ["mcp", "lsp-trust", "plugins"] {
        assert!(availability(&app, name).is_available(), "{name}");
    }
    assert_eq!(
        availability(&app, "workspace-init").reason(),
        Some(crate::service_health::WORKSPACE_PRESENT_REASON)
    );
    fs::remove_dir_all(root).unwrap();
}
