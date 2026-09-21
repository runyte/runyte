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
fn direct_keys_hints_and_help_report_platform_refusals() {
    use runyte::{
        command::{EditorCommand, GrammarKind, Mode},
        help::{self, HelpTopic},
        input::InputEvent,
        key_hints::KeyHintState,
        keymap::{Binding, BindingScope, Key, Keymap, default_keymap},
    };
    let root = TestRuntimeRoot::new("windows-keys").unwrap();
    let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
    let commands = [
        EditorCommand::ShellPipe,
        EditorCommand::OpenExplorerSystem,
        EditorCommand::Diagnostics,
    ];
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
    assert_eq!(rows.len(), 3);
    for (index, command) in commands.iter().enumerate() {
        let reason = runyte::command::CommandId::Editor(*command)
            .platform_unavailable()
            .unwrap();
        let row = rows
            .iter()
            .find(|row| row.target == Some((*command).into()))
            .unwrap();
        assert_eq!(row.unavailable_reason.as_deref(), Some(reason));
        app.handle_input(InputEvent::Key(Key::char('z'))).unwrap();
        app.handle_input(InputEvent::Key(Key::char((b'a' + index as u8) as char)))
            .unwrap();
        assert_eq!(app.status, reason);
    }
    let explorer = help::render(
        HelpTopic::Explorer,
        GrammarKind::Runyte,
        BindingScope::Directory,
        default_keymap(),
        false,
    );
    assert!(explorer.contains("External file opening is unavailable in Windows Phase 1"));
    let text = help::render(
        HelpTopic::Text,
        GrammarKind::Runyte,
        BindingScope::Global,
        default_keymap(),
        false,
    );
    assert!(text.contains("Shell filters are unavailable in Windows Phase 1"));
}

#[test]
fn deferred_commands_agree_with_palette_availability() {
    let root = TestRuntimeRoot::new("windows-commands").unwrap();
    let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
    for spelling in [
        "lsp-trust",
        "lsp-status",
        "plugins",
        "context-access",
        "git-status",
        "pipe echo text",
        "quit-here",
        "log-open",
        "session-list",
    ] {
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
    assert!(runyte::git::GitCliProvider::from_environment().is_none());
}

#[test]
fn binary_open_refuses_without_offering_an_unusable_program_prompt() {
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
    assert_eq!(app.mode, runyte::command::Mode::Normal);
    assert_eq!(app.active_buffer().to_string(), before);
    assert!(app.status.contains("External file opening is unavailable"));
}

#[tokio::test]
async fn enabled_lsp_configuration_stays_disabled_after_permission_and_restart() {
    let root = TestRuntimeRoot::new("windows-lsp").unwrap();
    let marker = root.path().join("unexpected-server-start");
    let mut config = Config::default().lsp;
    config.enable = true;
    config.servers.insert(
        "rust".into(),
        LanguageServerConfig {
            command: "cmd.exe".into(),
            args: vec![
                "/d".into(),
                "/c".into(),
                "echo started>unexpected-server-start".into(),
            ],
            ..Default::default()
        },
    );
    let (handle, mut events) = lsp::spawn(config, root.path().to_owned());
    handle.set_allowed(true);
    assert!(handle.send(LspCommand::Ensure {
        language: "rust".into()
    }));
    assert!(handle.send(LspCommand::Restart(Some("rust".into()))));
    assert!(handle.send(LspCommand::Status));
    // Status is processed after the ensure/restart requests, providing a barrier.
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let LspEvent::Status { message, .. } = event {
            if message.contains("no language servers running") {
                break;
            }
        } else {
            assert!(
                !matches!(event, LspEvent::Ready { .. } | LspEvent::Stopped { .. }),
                "{event:?}"
            );
        }
    }
    assert!(!marker.exists());
    assert!(handle.send(LspCommand::Shutdown));
}
