// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{HostPorts, plugin_workflows::Instance},
    clipboard::SystemClipboard,
    input::{InputEvent, KeyCode, KeyStroke, Modifiers},
    plugin::{ClientMessage, Event, HostMessage, PluginConfig, application as api},
    test_support::TestRuntimeRoot,
};
use anyhow::{Result, bail};
use tokio::sync::mpsc;

fn junction(link: &std::path::Path, target: &std::path::Path) {
    use std::{
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        },
    };

    std::fs::create_dir(link).unwrap();
    let junction = std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ | GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(link)
        .unwrap();
    let target = crate::windows_fs::ordinary_working_directory(target).unwrap();
    let substitute: Vec<_> = std::ffi::OsString::from(format!(r"\??\{}", target.display()))
        .encode_wide()
        .collect();
    let mut data = Vec::new();
    data.extend_from_slice(&0xA0000003u32.to_le_bytes());
    data.extend_from_slice(&((8 + (substitute.len() + 2) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
    data.extend_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    for unit in substitute {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data.extend_from_slice(&[0; 4]);
    let mut returned = 0;
    assert_ne!(
        unsafe {
            windows_sys::Win32::System::IO::DeviceIoControl(
                junction.as_raw_handle(),
                0x000900A4,
                data.as_ptr().cast(),
                data.len() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
}

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> Result<String> {
        bail!("inert")
    }
    fn write(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
}

fn fixture() -> (TestRuntimeRoot, WorkspaceHost, mpsc::Receiver<HostMessage>) {
    let root = TestRuntimeRoot::new("windows-plugin-handoffs").unwrap();
    let app = crate::app::App::new_in_isolated_project(
        root.path(),
        HostPorts::isolated(Box::new(Clipboard)),
    )
    .unwrap();
    let mut host = WorkspaceHost::new(app);
    let mut config = PluginConfig {
        settings: Default::default(),
        id: "native".into(),
        api: api::VERSION.to_owned(),
        runyte: format!("={}", crate::plugin::compatibility::HOST_VERSION),
        capabilities: vec!["external".into(), "terminals".into()],
        enabled: true,
        executable: "unstarted-native-plugin.exe".into(),
        args: vec![],
        bindings: Default::default(),
    };
    config.bindings.insert("open".into(), "F12".into());
    let (sender, receiver) = mpsc::channel(8);
    host.app.plugins.instances.insert(
        0,
        Instance {
            config,
            sender: crate::plugin::Sender::new(sender),
            registered: false,
            application: Default::default(),
        },
    );
    host.application_message(
        0,
        api::ClientMessage::Register {
            version: api::VERSION.into(),
            runyte: format!("={}", crate::plugin::compatibility::HOST_VERSION),
            settings_schema: None,
            name: "Native fixture".into(),
            help_topics: None,
            commands: vec![api::Registration {
                presentation: None,
                default_binding: None,
                alias: None,
                arguments: vec![],
                primary: false,
                name: "open".into(),
                description: "Open native resource".into(),
                context: api::CommandContext::Workspace,
            }],
            required_capabilities: ["external".into(), "terminals".into()].into(),
            optional_capabilities: Default::default(),
            required_features: Default::default(),
            optional_features: Default::default(),
        },
    )
    .unwrap();
    host.note_plugin_frontend(true);
    (root, host, receiver)
}

fn next(receiver: &mut mpsc::Receiver<HostMessage>) -> api::HostMessage {
    loop {
        match receiver.try_recv().unwrap() {
            HostMessage::Application(message) => return message,
            HostMessage::Deadline { .. } => {}
        }
    }
}

fn invocation(receiver: &mut mpsc::Receiver<HostMessage>) -> String {
    let api::HostMessage::Request { id, .. } = next(receiver) else {
        panic!("missing plugin invocation")
    };
    id
}

fn request(host: &mut WorkspaceHost, n: u64, request: api::Request) {
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Application(api::ClientMessage::Request {
            id: format!("p:{n}"),
            request,
        })),
    });
}

fn external(invocation: &str) -> api::Request {
    api::Request::ExternalOpen {
        invocation: invocation.into(),
        target: handoff::Target::Url {
            url: "https://example.test/native".into(),
        },
    }
}

fn failure(receiver: &mut mpsc::Receiver<HostMessage>) -> api::ErrorCode {
    let api::HostMessage::Response {
        outcome: api::Response::Failure { error },
        ..
    } = next(receiver)
    else {
        panic!("missing failure response")
    };
    error.code
}

fn type_command(host: &mut WorkspaceHost, suffix: InputEvent) {
    for character in ":plugin.native.open".chars() {
        host.app
            .handle_input(KeyStroke::char(character).into())
            .unwrap();
    }
    host.app.handle_input(suffix).unwrap();
}

async fn settle(host: &mut WorkspaceHost, events: &mut mpsc::Receiver<Event>) {
    while !host.plugin_handoffs.is_empty() {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        host.handle_plugin_event(event);
    }
}

#[tokio::test]
async fn native_handoff_requires_current_nonreplayed_unmodified_physical_enter() {
    let (_root, mut host, mut output) = fixture();
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Registered { .. }
    ));

    let semantic = host.app.parse_command("plugin.native.open").unwrap();
    host.app.execute(semantic).unwrap();
    let semantic = invocation(&mut output);
    request(&mut host, 1, external(&semantic));
    assert_eq!(failure(&mut output), api::ErrorCode::ContextChanged);

    host.app
        .handle_input(InputEvent::Key(KeyStroke::new(
            KeyCode::Function(12),
            Modifiers::NONE,
        )))
        .unwrap();
    let binding = invocation(&mut output);
    let binding_context = &host.app.plugins.instances[&0].application.requests[&binding];
    assert!(binding_context.foreground_allowed);
    assert!(!binding_context.native_handoff_allowed);
    host.app.plugin_foreground(binding_context).unwrap();
    request(&mut host, 2, external(&binding));
    assert_eq!(failure(&mut output), api::ErrorCode::ContextChanged);

    type_command(
        &mut host,
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::SHIFT)),
    );
    let modified = invocation(&mut output);
    request(&mut host, 3, external(&modified));
    assert_eq!(failure(&mut output), api::ErrorCode::ContextChanged);

    for character in ":plugin.native.open".chars() {
        host.app
            .handle_input(KeyStroke::char(character).into())
            .unwrap();
    }
    host.app
        .handle_repeated_input(InputEvent::Key(KeyStroke::new(
            KeyCode::Enter,
            Modifiers::NONE,
        )))
        .unwrap();
    let replayed = invocation(&mut output);
    request(&mut host, 4, external(&replayed));
    assert_eq!(failure(&mut output), api::ErrorCode::ContextChanged);

    type_command(
        &mut host,
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)),
    );
    let physical = invocation(&mut output);
    host.app
        .handle_repeated_input(InputEvent::Key(KeyStroke::char('x')))
        .unwrap();
    request(&mut host, 5, external(&physical));
    assert_eq!(failure(&mut output), api::ErrorCode::ContextChanged);

    type_command(
        &mut host,
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)),
    );
    let current = invocation(&mut output);
    let (sender, mut events) = mpsc::channel(crate::plugin::EVENT_CAPACITY);
    host.plugin_events_sender = Some(sender);
    host.plugin_external_launcher = Some(
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
    );
    request(&mut host, 6, external(&current));
    settle(&mut host, &mut events).await;
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Response {
            outcome: api::Response::Success { .. },
            ..
        }
    ));
    request(&mut host, 7, external(&current));
    assert_eq!(failure(&mut output), api::ErrorCode::ContextChanged);
}

#[tokio::test]
async fn native_terminal_install_transfers_ownership_past_plugin_stop() {
    let (_root, mut host, mut output) = fixture();
    next(&mut output);
    type_command(
        &mut host,
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)),
    );
    let invocation = invocation(&mut output);
    let (sender, mut events) = mpsc::channel(crate::plugin::EVENT_CAPACITY);
    host.plugin_events_sender = Some(sender);
    request(
        &mut host,
        1,
        api::Request::TerminalOpen(handoff::TerminalOpen {
            invocation,
            label: "Native fixture".into(),
            executable: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: vec![
                "--exact".into(),
                "terminal::pty::tests::native_console_fixture".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: None,
        }),
    );
    settle(&mut host, &mut events).await;
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Response {
            outcome: api::Response::Success {
                result: api::ResultValue::TerminalOpened { .. }
            },
            ..
        }
    ));
    let terminal = host.app.active_terminal().unwrap();
    host.stop_plugin(0, "fixture stop");
    assert!(
        host.app
            .terminals
            .get(terminal)
            .is_some_and(|session| session.live())
    );
    let cleanup = host
        .app
        .terminals
        .get(terminal)
        .and_then(|session| session.cleanup_waiter())
        .unwrap();
    assert!(host.app.terminals.close(terminal));
    cleanup();
}

#[tokio::test]
async fn native_terminal_cwd_cannot_escape_workspace_or_follow_junction() {
    let (root, mut host, mut output) = fixture();
    next(&mut output);
    let outside = TestRuntimeRoot::new("windows-plugin-handoff-outside").unwrap();
    let link = root.join("escape");
    junction(&link, outside.path());
    let (sender, mut events) = mpsc::channel(crate::plugin::EVENT_CAPACITY);
    host.plugin_events_sender = Some(sender);

    for (request_id, cwd) in [(1, ".."), (2, "escape")] {
        type_command(
            &mut host,
            InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)),
        );
        let invocation = invocation(&mut output);
        request(
            &mut host,
            request_id,
            api::Request::TerminalOpen(handoff::TerminalOpen {
                invocation,
                label: "Refused native fixture".into(),
                executable: std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                args: vec![],
                cwd: Some(cwd.into()),
            }),
        );
        settle(&mut host, &mut events).await;
        assert_eq!(failure(&mut output), api::ErrorCode::InvalidArgument);
        assert_eq!(host.app.terminals.len(), 0);
    }
    std::fs::remove_dir(&link).unwrap();
}

#[tokio::test]
async fn native_late_result_rejects_replaced_attachment_and_stopped_plugin_generation() {
    let (_root, mut host, mut output) = fixture();
    next(&mut output);
    let (sender, mut events) = mpsc::channel(crate::plugin::EVENT_CAPACITY);
    host.plugin_events_sender = Some(sender);

    type_command(
        &mut host,
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)),
    );
    let replaced_attachment = invocation(&mut output);
    request(&mut host, 1, external(&replaced_attachment));
    let prepared = events.recv().await.unwrap();
    host.note_plugin_frontend(false);
    host.note_plugin_frontend(true);
    host.handle_plugin_event(prepared);
    settle(&mut host, &mut events).await;
    assert_eq!(failure(&mut output), api::ErrorCode::ContextChanged);

    type_command(
        &mut host,
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)),
    );
    let stopped_generation = invocation(&mut output);
    request(&mut host, 2, external(&stopped_generation));
    let prepared = events.recv().await.unwrap();
    host.stop_plugin(0, "generation replaced during native handoff");
    assert_eq!(host.app.plugins.orphaned_payload, 128 * 1024);
    host.handle_plugin_event(prepared);
    settle(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.orphaned_payload, 0);
}
