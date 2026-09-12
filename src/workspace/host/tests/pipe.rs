// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, HostPorts},
    clipboard::SystemClipboard,
    command::{CommandExecutionContext, CommandInvocation, EditorCommand, parse_colon_command},
    selection::{Range, Selection},
    test_support::TestRuntimeRoot,
};

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        anyhow::bail!("inert")
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
fn host(
    text: &str,
) -> (
    TestRuntimeRoot,
    WorkspaceHost,
    tokio::sync::mpsc::Receiver<Completion>,
) {
    let root = TestRuntimeRoot::new("pipe").unwrap();
    let app = App::new_in_isolated_project(root.path(), HostPorts::isolated(Box::new(Clipboard)))
        .unwrap();
    let mut host = WorkspaceHost::new(app);
    host.app.apply_to_buffer(0, &Transaction::insert(0, text));
    host.app.buffers[0].commit_undo_group();
    host.app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(
        0,
        host.app.buffers[0].row_end_offset(host.app.buffers[0].last_row(), false),
    ));
    let receiver = host.start_pipe_service();
    (root, host, receiver)
}
fn invoke(host: &mut WorkspaceHost, command: &str) {
    host.app
        .execute(parse_colon_command(command).unwrap())
        .unwrap();
    host.sync_pipe();
}
async fn finish(host: &mut WorkspaceHost, receiver: &mut tokio::sync::mpsc::Receiver<Completion>) {
    let completion = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    host.handle_pipe_completion(completion);
}
fn undo(host: &mut WorkspaceHost) {
    host.app
        .execute(
            CommandInvocation::editor(EditorCommand::Undo, CommandExecutionContext::default())
                .unwrap(),
        )
        .unwrap();
}

#[tokio::test]
async fn both_spellings_preserve_newlines_and_undo_once_without_a_frame() {
    for command in ["pipe sort", "| sort"] {
        let (_root, mut host, mut receiver) = host("b\na\n");
        invoke(&mut host, command);
        finish(&mut host, &mut receiver).await;
        assert_eq!(host.app.buffers[0].to_string(), "a\nb\n");
        assert!(host.current_frame_id().is_none());
        undo(&mut host);
        assert_eq!(host.app.buffers[0].to_string(), "b\na\n");
    }
}

#[tokio::test]
async fn unicode_reversed_selections_remain_independent_across_panes_and_attachments() {
    let (root, mut host, mut receiver) = host("éß 😀xy");
    host.app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::new(1, 0), Range::new(3, 5)], 1);
    invoke(&mut host, "pipe cat; printf '!\\n'");
    host.note_plugin_frontend(false);
    host.sync_plugin_observers();
    host.app
        .execute(parse_colon_command("vsplit").unwrap())
        .unwrap();
    let path = root.join("other.txt");
    std::fs::write(&path, "other").unwrap();
    let other = host.open_buffer(path, true).unwrap();
    finish(&mut host, &mut receiver).await;
    host.note_plugin_frontend(true);
    host.sync_plugin_observers();
    assert_eq!(host.app.buffers[0].to_string(), "éß!\n 😀xy!\n");
    assert_eq!(BufferId::from_index(host.app.active().buffer), other);
    assert_eq!(host.app.active_buffer().to_string(), "other");
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn stale_closed_readonly_and_cancelled_results_never_apply() {
    for scenario in ["edit", "undo", "closed", "readonly", "cancel"] {
        let (_root, mut host, mut receiver) = host("abc");
        invoke(&mut host, "pipe printf changed");
        let completion = receiver.recv().await.unwrap();
        match scenario {
            "edit" | "undo" => {
                host.app.apply_to_buffer(0, &Transaction::insert(3, "!"));
                if scenario == "undo" {
                    undo(&mut host);
                }
            }
            "closed" => host.close_buffer(BufferId::from_index(0), true).unwrap(),
            "readonly" => host.app.buffers[0].kind = crate::buffer::BufferKind::Help,
            "cancel" => invoke(&mut host, "pipe-cancel"),
            _ => unreachable!(),
        }
        let before = host.app.buffers[0].to_string();
        host.handle_pipe_completion(completion);
        assert_eq!(host.app.buffers[0].to_string(), before, "{scenario}");
        assert!(host.app.pipe.cancellation.is_none());
    }
}

#[tokio::test]
async fn failed_selection_is_atomic_and_empty_output_deletes() {
    let (_root, mut host, mut receiver) = host("a b");
    host.app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::new(0, 0), Range::new(2, 2)], 0);
    invoke(
        &mut host,
        "pipe input=$(cat); if [ \"$input\" = b ]; then echo failed >&2; exit 3; fi; printf changed",
    );
    finish(&mut host, &mut receiver).await;
    assert_eq!(host.app.buffers[0].to_string(), "a b");
    invoke(&mut host, "| cat >/dev/null");
    finish(&mut host, &mut receiver).await;
    assert_eq!(host.app.buffers[0].to_string(), " ");
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "a b");
}

#[tokio::test]
async fn admission_and_shutdown_bound_the_workspace_job() {
    let (_root, mut host, mut receiver) = host("abc");
    invoke(&mut host, "pipe sleep 30");
    assert!(matches!(
        host.app
            .execute(parse_colon_command("pipe cat").unwrap())
            .unwrap(),
        crate::app::CommandOutcome::UserError(_)
    ));
    host.shutdown_pipe().await;
    assert!(
        receiver
            .recv()
            .await
            .unwrap()
            .0
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(host.pipe_worker.is_none());
}

#[tokio::test]
async fn command_quotes_trailing_escapes_and_captured_directory_reach_the_shell() {
    let (root, mut host, mut receiver) = host("abc");
    let invocation = host
        .app
        .parse_command("pipe printf '%s' escaped\\ ")
        .unwrap();
    host.app.execute(invocation).unwrap();
    host.sync_pipe();
    finish(&mut host, &mut receiver).await;
    assert_eq!(host.app.buffers[0].to_string(), "escaped ");
    host.app
        .execute(CommandInvocation::editor(EditorCommand::SelectAll, Default::default()).unwrap())
        .unwrap();
    let invocation = host.app.parse_command("| pwd").unwrap();
    host.app.execute(invocation).unwrap();
    host.app.project_root = root.join("not-the-captured-root");
    host.sync_pipe();
    finish(&mut host, &mut receiver).await;
    assert_eq!(
        host.app.buffers[0].to_string(),
        format!("{}\n", root.path().display())
    );
}

#[tokio::test]
async fn empty_buffer_empty_output_is_success_and_admission_limits_do_not_queue_work() {
    let (_root, mut host, mut receiver) = host("");
    invoke(&mut host, "pipe cat");
    finish(&mut host, &mut receiver).await;
    assert_eq!(host.app.status, "Pipe completed");
    for command in ["pipe".to_owned(), format!("pipe {}", "x".repeat(16385))] {
        if let Ok(invocation) = host.app.parse_command(&command) {
            assert!(matches!(
                host.app.execute(invocation).unwrap(),
                crate::app::CommandOutcome::UserError(_)
            ));
        }
        assert!(host.app.pipe.request.is_none());
    }
    host.app.buffers[0].kind = crate::buffer::BufferKind::Help;
    assert!(matches!(
        host.app
            .execute(parse_colon_command("pipe cat").unwrap())
            .unwrap(),
        crate::app::CommandOutcome::UserError(_)
    ));
    assert!(host.app.pipe.request.is_none());
}
