// SPDX-License-Identifier: MPL-2.0

#![cfg(windows)]

use runyte::{
    app::{App, CommandOutcome},
    command::{parse_colon_command, resolve_command},
    config::{Config, LanguageServerConfig},
    lsp::{self, LspCommand, LspEvent},
    test_support::TestRuntimeRoot,
    workspace::WorkspaceHost,
};

#[test]
fn native_explorer_opening_agrees_with_keys_hints_and_help() {
    use runyte::{
        command::{EditorCommand, GrammarKind, Mode},
        help::{self, HelpTopic},
        input::InputEvent,
        key_hints::KeyHintState,
        keymap::{Binding, BindingScope, Key, Keymap, default_keymap},
    };
    let root = TestRuntimeRoot::new("windows-keys").unwrap();
    let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
    let commands = [EditorCommand::OpenExplorerSystem];
    let bindings = commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            Binding::implemented(
                &[Mode::Normal],
                [Key::char('z'), Key::char((b'a' + index as u8) as char)],
                *command,
            )
        })
        .collect();
    let keymap = std::sync::Arc::new(Keymap::new(bindings).unwrap());
    app.set_keymap(keymap.clone());
    let mut hints = KeyHintState::default();
    hints.observe(Key::char('z'), Mode::Normal, &keymap);
    let mut rows = hints.rows(&keymap, Mode::Normal);
    for row in &mut rows {
        row.apply_capabilities(&app.command_capabilities());
    }
    assert_eq!(rows.len(), commands.len());
    for (index, command) in commands.iter().enumerate() {
        assert!(
            runyte::command::CommandId::Editor(*command)
                .platform_unavailable()
                .is_none()
        );
        let row = rows
            .iter()
            .find(|row| row.target == Some((*command).into()))
            .unwrap();
        assert!(row.unavailable_reason.is_none());
        app.handle_input(InputEvent::Key(Key::char('z'))).unwrap();
        app.handle_input(InputEvent::Key(Key::char((b'a' + index as u8) as char)))
            .unwrap();
        assert!(app.status.contains("not a directory buffer"));
    }
    let explorer = help::render(
        HelpTopic::Explorer,
        GrammarKind::Runyte,
        BindingScope::Directory,
        default_keymap(),
        false,
    );
    assert!(!explorer.contains("External file opening is unavailable in Windows Phase 1"));
    let text = help::render(
        HelpTopic::Text,
        GrammarKind::Runyte,
        BindingScope::Global,
        default_keymap(),
        false,
    );
    assert!(!text.contains("Shell filters are unavailable in Windows Phase 1"));
}

#[test]
fn deferred_commands_agree_with_palette_availability() {
    let root = TestRuntimeRoot::new("windows-commands").unwrap();
    let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
    for spelling in ["plugins", "context-access", "session-attach workspace"] {
        let name = spelling.split_whitespace().next().unwrap();
        let spec = resolve_command(name).unwrap();
        let availability = app.command_capabilities().command_availability(spec);
        let reason = availability
            .reason()
            .expect("deferred command is unavailable");
        let result = app.execute(parse_colon_command(spelling).unwrap()).unwrap();
        assert!(
            matches!(result, CommandOutcome::Unavailable(ref message) if message == reason),
            "{spelling}: {result:?}"
        );
    }
    let manager = resolve_command("session-list").unwrap();
    assert!(manager.id.platform_unavailable().is_none());
    let reason = app
        .command_capabilities()
        .command_availability(manager)
        .reason()
        .unwrap()
        .to_owned();
    assert_eq!(reason, "session service is unavailable");
    let result = app
        .execute(parse_colon_command("session-list").unwrap())
        .unwrap();
    assert!(matches!(result, CommandOutcome::UserError(message) if message == reason));
    assert!(!app.should_quit);
    assert!(
        runyte::command::CommandId::Editor(runyte::command::EditorCommand::MatchBracket)
            .platform_unavailable()
            .is_none()
    );
    assert!(
        runyte::command::CommandId::Editor(runyte::command::EditorCommand::DocumentOutline)
            .platform_unavailable()
            .is_none()
    );
    for spelling in ["lsp-trust", "lsp-status", "lsp-restart"] {
        let spec = resolve_command(spelling).unwrap();
        assert!(spec.id.platform_unavailable().is_none());
    }
    assert!(
        runyte::command::CommandId::Editor(runyte::command::EditorCommand::Diagnostics)
            .platform_unavailable()
            .is_none()
    );
}

#[test]
fn enabled_plugins_cannot_start_even_without_a_runtime() {
    let root = TestRuntimeRoot::new("windows-plugins").unwrap();
    let config: Config = serde_yaml::from_str(
        "plugins:\n  - id: example\n    enabled: true\n    executable: cmd.exe\n",
    )
    .unwrap();
    let app = App::new_in_project(config, None, root.path()).unwrap();
    let mut host = WorkspaceHost::new(app);
    assert!(host.start_plugins().is_none());
    assert!(host.start_plugins().is_none());
}

#[test]
fn binary_open_offers_a_program_prompt_without_loading_binary_text() {
    let root = TestRuntimeRoot::new("windows-binary").unwrap();
    let path = root.path().join("image.bin");
    std::fs::write(&path, [0, 1, 2, 3]).unwrap();
    let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
    let before = app.active_buffer().to_string();
    app.execute(
        runyte::command::CommandInvocation::from_parts(
            runyte::command::CommandId::Colon(runyte::command::ColonCommand::Open),
            runyte::command::InvocationParameters::Path(path),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(app.mode, runyte::command::Mode::Command);
    assert_eq!(app.prompt_kind, runyte::app::PromptKind::ExternalProgram);
    assert_eq!(app.active_buffer().to_string(), before);
    assert!(app.status.contains("not a text file"));
}

#[tokio::test]
async fn native_lsp_requires_permission_and_missing_servers_fail_nonfatally_after_restart() {
    use runyte::input::{InputEvent, KeyCode, KeyStroke};
    let root = TestRuntimeRoot::new("windows-lsp").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let file = project.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let mut config = Config::default();
    config.lsp.enable = true;
    config.lsp.servers.insert(
        "rust".into(),
        LanguageServerConfig {
            command: root.join("missing-native-language-server.exe"),
            ..Default::default()
        },
    );
    let (handle, mut events) = lsp::spawn(config.lsp.clone(), project.clone());
    let mut app = App::new_in_project(config, Some(file), &project).unwrap();
    app.configure_lsp_trust(Some(root.join("cache/lsp-trust")));
    app.attach_lsp(handle.clone());
    assert_eq!(
        app.command_capabilities().lsp_manager.reason(),
        Some("LSP is disabled for this workspace; use :lsp-trust")
    );
    assert!(
        app.command_capabilities()
            .command_availability(resolve_command("lsp-trust").unwrap())
            .is_available()
    );
    assert!(matches!(
        app.execute(parse_colon_command("lsp-status").unwrap())
            .unwrap(),
        CommandOutcome::Unavailable(_)
    ));
    // Physical choice input grants this run; attaching the manager and asking
    // for status above cannot grant permission or queue a server start.
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Down)))
        .unwrap();
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.command_capabilities().lsp_manager.is_available());
    for attempt in 0..2 {
        if attempt != 0 {
            app.execute(parse_colon_command("lsp-restart").unwrap())
                .unwrap();
            assert!(handle.send(LspCommand::Ensure {
                language: "rust".into()
            }));
        }
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.expect("manager remains alive");
                assert!(!matches!(event, LspEvent::Ready { .. }), "{event:?}");
                if matches!(event, LspEvent::Stopped { .. }) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert!(matches!(&event, LspEvent::Stopped { language, message }
            if language == "rust" && message.contains("cannot start missing-native-language-server.exe")));
        app.apply_lsp_event(event);
        assert!(!app.should_quit);
        assert!(app.command_capabilities().lsp_manager.is_available());
        assert_eq!(app.active_buffer().to_string(), "fn main() {}\n");
    }
    app.handle_input(InputEvent::Key(KeyStroke::char('i')))
        .unwrap();
    app.handle_input(InputEvent::Text("// still editing\n".into()))
        .unwrap();
    assert!(app.active_buffer().to_string().contains("// still editing"));
    assert!(handle.send(LspCommand::Shutdown));
}
