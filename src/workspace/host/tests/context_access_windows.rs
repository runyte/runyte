// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, HostPorts},
    clipboard::SystemClipboard,
    input::{InputEvent, KeyCode, KeyStroke},
    terminal::{TerminalOutput, TerminalRequest, proposal::DeliveryState},
    test_support::TestRuntimeRoot,
    workspace::context::storage::{HostMode, StorageLocation, random_token},
};
use std::os::windows::ffi::OsStringExt;

const TERMINAL_FIXTURE: &str = "workspace::host::context::tests::compiled_context_terminal_child";

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        anyhow::bail!("inert fixture")
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn host(project: &std::path::Path) -> WorkspaceHost {
    WorkspaceHost::new(
        App::new_in_isolated_project(project, HostPorts::isolated(Box::new(Clipboard))).unwrap(),
    )
}

fn key(host: &mut WorkspaceHost, code: KeyCode) {
    host.app
        .handle_input(InputEvent::Key(KeyStroke::plain(code)))
        .unwrap();
}

fn review_all(host: &mut WorkspaceHost) {
    let pages = host.app.context_ui.surface.as_ref().unwrap().last_page() + 1;
    for frame in 1..=pages as u64 {
        host.app.note_context_frame(80, 22, frame);
        host.app.note_context_presented(frame);
        if host
            .app
            .context_ui
            .surface
            .as_ref()
            .unwrap()
            .fully_reviewed()
        {
            return;
        }
        key(host, KeyCode::Char('j'));
    }
    panic!("context review did not finish within {pages} pages");
}

fn frame(host: &mut WorkspaceHost, connection: u64, value: Value) -> Reply {
    host.context_frame(
        connection,
        serde_json::to_string(&value).unwrap().as_bytes(),
    )
}

fn authenticate(host: &mut WorkspaceHost, identity: &Identity, connection: u64) {
    assert_eq!(
        frame(
            host,
            connection,
            json!({"type":"authenticate","credential":identity.credential()})
        )
        .value["type"],
        "hello"
    );
    assert_eq!(
        frame(
            host,
            connection,
            json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"windows-host-fixture","commands":[],"required_capabilities":["terminal_read","terminal_propose"],"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":[]})
        )
        .value["type"],
        "registered"
    );
}

#[test]
#[ignore = "launched inside the native Windows context ConPTY test"]
fn compiled_context_terminal_child() {
    use std::io::{BufRead, Write};

    println!("READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).unwrap();
    println!("SUBMITTED:{}", line.trim_end_matches(['\r', '\n']));
    std::io::stdout().flush().unwrap();
}

#[test]
fn normal_windows_host_keeps_context_access_inert_until_private_start() {
    let root = TestRuntimeRoot::new("context-host-gated").unwrap();
    let mut host = host(root.path());
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    assert!(host.app.context_ui.requested_identity.is_none());
    assert!(host.app.context_ui.surface.is_none());
    assert!(!host.context_enabled());
    assert!(host.context_delay().is_none());
}

#[tokio::test]
async fn remembered_grants_are_staged_then_reopened_writable() {
    let root = TestRuntimeRoot::new("context-host-remembered").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let context_root = root.path().join("context");
    let location = StorageLocation::explicit(context_root.clone()).unwrap();
    let environment = random_token().unwrap();

    let mut first = host(&project);
    let _events = first
        .start_context_windows(
            HostMode::Standalone,
            Some(location.clone()),
            environment.clone(),
        )
        .unwrap();
    first.context_open_storage(context_root.clone()).unwrap();
    let storage = first.context.storage.as_ref().unwrap().clone();
    let identity = storage.identity("agent").unwrap();
    storage
        .grant(
            &first.app.project_root,
            &identity,
            [Scope::EditorContextRead].into(),
        )
        .unwrap();
    first.shutdown_context().await.unwrap();

    let mut second = host(&project);
    let _events = second
        .start_context_windows(HostMode::Persistent, Some(location), environment.clone())
        .unwrap();
    assert_eq!(
        second.context.grants[&identity.fingerprint()].1,
        [Scope::EditorContextRead].into()
    );
    assert!(second.context_enabled());
    assert_eq!(
        second.context.registration.as_ref().unwrap().mode,
        HostMode::Persistent
    );
    let reopened = second.context.storage.as_ref().unwrap().clone();
    assert_eq!(reopened.discover(&environment, false).unwrap().len(), 1);
    second.shutdown_context().await.unwrap();
    assert!(reopened.discover(&environment, true).unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_host_shutdown_retains_owner_for_retry() {
    use std::{future::poll_fn, task::Poll};
    let root = TestRuntimeRoot::new("context-host-shutdown-cancel").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let context_root = root.path().join("context");
    let location = StorageLocation::explicit(context_root).unwrap();
    let environment = random_token().unwrap();
    let mut host = host(&project);
    let _events = host
        .start_context_windows(HostMode::Standalone, Some(location), environment.clone())
        .unwrap();
    host.context_open_storage(root.path().join("context"))
        .unwrap();
    let storage = host.context.storage.as_ref().unwrap().clone();
    let identity = storage.identity("agent").unwrap();
    host.context.grants.insert(
        identity.fingerprint(),
        (identity, [Scope::EditorContextRead].into()),
    );
    host.context_enable().unwrap();
    let mut shutdown = Box::pin(host.shutdown_context());
    poll_fn(|cx| {
        let _ = shutdown.as_mut().poll(cx);
        Poll::Ready(())
    })
    .await;
    drop(shutdown);
    assert!(host.context.stopping);
    assert!(host.context.storage.is_some());
    host.shutdown_context().await.unwrap();
    assert!(host.context.storage.is_none());
    assert!(storage.discover(&environment, true).unwrap().is_empty());
}

#[tokio::test]
async fn native_grants_default_to_reject_and_only_remembered_access_reopens() {
    let root = TestRuntimeRoot::new("context-host-native-grants").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let context_root = root.path().join("context");
    let location = StorageLocation::explicit(context_root.clone()).unwrap();
    let environment = random_token().unwrap();

    let mut first = host(&project);
    let _events = first
        .start_context_windows(
            HostMode::Standalone,
            Some(location.clone()),
            environment.clone(),
        )
        .unwrap();
    first.note_plugin_frontend(true);
    first.app.context_ui.requested_identity = Some("agent".into());
    first.sync_context();
    assert!(
        !first
            .app
            .context_ui
            .surface
            .as_ref()
            .unwrap()
            .accept_selected
    );
    review_all(&mut first);
    key(&mut first, KeyCode::Enter);
    first.sync_context();
    assert!(!first.context_enabled());

    first.app.context_ui.requested_identity = Some("agent".into());
    first.sync_context();
    review_all(&mut first);
    key(&mut first, KeyCode::Tab);
    key(&mut first, KeyCode::Enter);
    first.sync_context();
    assert!(first.context_enabled());
    let storage = first.context.storage.as_ref().unwrap().clone();
    let identity = storage.identity("agent").unwrap();
    assert!(storage.scopes(&project, &identity).unwrap().is_empty());
    first.shutdown_context().await.unwrap();

    let mut second = host(&project);
    let _events = second
        .start_context_windows(
            HostMode::Persistent,
            Some(location.clone()),
            environment.clone(),
        )
        .unwrap();
    assert!(!second.context_enabled());
    second.note_plugin_frontend(true);
    second.app.context_ui.requested_identity = Some("agent".into());
    second.sync_context();
    review_all(&mut second);
    key(&mut second, KeyCode::Char('r'));
    key(&mut second, KeyCode::Tab);
    key(&mut second, KeyCode::Enter);
    second.sync_context();
    assert!(second.context_enabled());
    assert_eq!(
        second
            .context
            .storage
            .as_ref()
            .unwrap()
            .scopes(&project, &identity)
            .unwrap(),
        [Scope::TerminalRead, Scope::EditorContextRead].into()
    );
    second.shutdown_context().await.unwrap();

    let mut third = host(&project);
    let _events = third
        .start_context_windows(HostMode::Persistent, Some(location), environment.clone())
        .unwrap();
    assert!(third.context_enabled());
    let registration = third.context.registration.clone().unwrap();
    third.note_plugin_frontend(true);
    third.app.context_ui.requested_identity = Some("agent".into());
    third.sync_context();
    key(&mut third, KeyCode::Char('x'));
    third.sync_context();
    assert!(!third.context_enabled());
    assert!(third.context.readers.is_empty());
    assert!(storage.scopes(&project, &identity).unwrap().is_empty());
    third.shutdown_context().await.unwrap();
    assert!(storage.discover(&environment, true).unwrap().is_empty());
    assert_ne!(registration.host_incarnation, "");
}

#[tokio::test]
async fn native_proposal_approval_rejects_nonphysical_and_stale_attempts() {
    let root = TestRuntimeRoot::new("context-host-approval-input").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let context_root = root.path().join("context");
    let mut host = host(&project);
    let _events = host
        .start_context_windows(
            HostMode::Standalone,
            Some(StorageLocation::explicit(context_root.clone()).unwrap()),
            random_token().unwrap(),
        )
        .unwrap();
    host.context_open_storage(context_root).unwrap();
    let identity = host
        .context
        .storage
        .as_ref()
        .unwrap()
        .identity("agent")
        .unwrap();
    host.context.grants.insert(
        identity.fingerprint(),
        (
            identity.clone(),
            [Scope::TerminalRead, Scope::TerminalPropose].into(),
        ),
    );
    host.context_enable().unwrap();
    host.note_plugin_frontend(true);
    authenticate(&mut host, &identity, 7);
    let terminal = host.app.terminals.insert_test_session(20, 3);

    let proposal = host.context_propose(7, terminal, "literal", None).unwrap()["proposal"]
        .as_str()
        .unwrap()
        .to_owned();
    host.sync_context();
    review_all(&mut host);
    key(&mut host, KeyCode::Tab);
    host.app
        .handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    host.app
        .handle_input(InputEvent::Text("\r\n".into()))
        .unwrap();
    host.app
        .handle_input(InputEvent::Key(KeyStroke::ctrl('m')))
        .unwrap();
    assert_eq!(host.context.proposals[&proposal].status, "pending");
    assert!(host.app.context_ui.decision.is_none());

    host.app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .send_text("changed");
    key(&mut host, KeyCode::Enter);
    host.sync_context();
    assert_eq!(host.context.proposals[&proposal].status, "stale");

    let stale_attachment = host.context_propose(7, terminal, "second", None).unwrap()["proposal"]
        .as_str()
        .unwrap()
        .to_owned();
    host.sync_context();
    review_all(&mut host);
    key(&mut host, KeyCode::Tab);
    host.note_plugin_frontend(false);
    host.note_plugin_frontend(true);
    key(&mut host, KeyCode::Enter);
    host.sync_context();
    assert_eq!(
        host.context.proposals[&stale_attachment].status,
        "cancelled"
    );

    host.note_plugin_frontend(false);
    assert_eq!(
        host.context_propose(7, terminal, "detached", None)
            .unwrap_err()
            .code,
        Code::NoFrontend
    );
    host.shutdown_context().await.unwrap();
}

#[tokio::test]
async fn conpty_approval_inserts_exactly_once_and_a_separate_enter_submits() {
    let root = TestRuntimeRoot::new("context-host-conpty-approval").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let context_root = root.path().join("context");
    let mut host = host(&project);
    let _events = host
        .start_context_windows(
            HostMode::Standalone,
            Some(StorageLocation::explicit(context_root.clone()).unwrap()),
            random_token().unwrap(),
        )
        .unwrap();
    host.context_open_storage(context_root).unwrap();
    let identity = host
        .context
        .storage
        .as_ref()
        .unwrap()
        .identity("agent")
        .unwrap();
    host.context.grants.insert(
        identity.fingerprint(),
        (
            identity.clone(),
            [Scope::TerminalRead, Scope::TerminalPropose].into(),
        ),
    );
    host.context_enable().unwrap();
    host.note_plugin_frontend(true);
    authenticate(&mut host, &identity, 9);

    let terminal = host
        .app
        .terminals
        .open(
            TerminalRequest {
                program: std::env::current_exe().unwrap().into_os_string(),
                arguments: vec![
                    "--exact".into(),
                    TERMINAL_FIXTURE.into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                ],
                directory: project,
                label: "context approval fixture".into(),
            },
            80,
            20,
        )
        .unwrap();
    let cleanup = host
        .app
        .terminals
        .get(terminal)
        .and_then(|session| session.cleanup_waiter())
        .unwrap();
    let mut output = host.take_terminal_events().unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !String::from_utf8_lossy(&bytes).contains("READY") {
            let event = output.recv().await.unwrap();
            if let TerminalOutput::Bytes { bytes: part, .. } = &event {
                bytes.extend(part);
            }
            host.app.terminals.apply(event);
        }
    })
    .await
    .unwrap();

    let proposal = host
        .context_propose(9, terminal, "echo exact", None)
        .unwrap()["proposal"]
        .as_str()
        .unwrap()
        .to_owned();
    host.sync_context();
    review_all(&mut host);
    key(&mut host, KeyCode::Tab);
    key(&mut host, KeyCode::Enter);
    host.sync_context();
    let delivery = host.context.proposals[&proposal]
        .delivery
        .as_ref()
        .unwrap()
        .clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        while delivery.state() != DeliveryState::Delivered {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    while let Ok(Some(event)) = tokio::time::timeout_at(quiet_deadline, output.recv()).await {
        if let TerminalOutput::Bytes { bytes: part, .. } = &event {
            bytes.extend(part);
        }
        host.app.terminals.apply(event);
    }
    assert!(
        !String::from_utf8_lossy(&bytes).contains("SUBMITTED:"),
        "approval Enter reached the child"
    );
    assert!(
        host.app
            .terminals
            .get_mut(terminal)
            .unwrap()
            .send_key(KeyStroke::plain(KeyCode::Enter))
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while !String::from_utf8_lossy(&bytes).contains("SUBMITTED:echo exact") {
            let event = output.recv().await.unwrap();
            if let TerminalOutput::Bytes { bytes: part, .. } = &event {
                bytes.extend(part);
            }
            host.app.terminals.apply(event);
        }
    })
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&bytes)
            .matches("SUBMITTED:echo exact")
            .count(),
        1
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = output.recv().await.unwrap();
            let exited = matches!(
                &event,
                TerminalOutput::Exited {
                    id,
                    code: Some(0)
                } if *id == terminal
            );
            host.app.terminals.apply(event);
            if exited {
                break;
            }
        }
    })
    .await
    .unwrap();
    host.sync_context();
    assert_eq!(host.context.proposals[&proposal].status, "delivered");
    assert!(host.app.terminals.close(terminal));
    cleanup();
    host.shutdown_context().await.unwrap();
}

#[test]
fn private_start_rejects_workspace_storage_overlap() {
    let root = TestRuntimeRoot::new("context-host-overlap").unwrap();
    let mut host = host(root.path());
    let context_root = root.path().join("context");
    let _events = host
        .start_context_windows(
            HostMode::Standalone,
            Some(StorageLocation::explicit(context_root.clone()).unwrap()),
            random_token().unwrap(),
        )
        .unwrap();
    assert!(host.context_open_storage(context_root).is_err());
    assert!(!host.context_enabled());
}

#[test]
fn private_start_rejects_non_unicode_workspace_root() {
    let root = TestRuntimeRoot::new("context-host-non-unicode").unwrap();
    let project = root
        .path()
        .join(std::ffi::OsString::from_wide(&[b'p' as u16, 0xd800]));
    std::fs::create_dir(&project).unwrap();
    let mut host = host(&project);
    let context_root = root.path().join("context");
    let _events = host
        .start_context_windows(
            HostMode::Standalone,
            Some(StorageLocation::explicit(context_root.clone()).unwrap()),
            random_token().unwrap(),
        )
        .unwrap();
    assert!(host.context_open_storage(context_root).is_err());
    assert!(!host.context_enabled());
}
