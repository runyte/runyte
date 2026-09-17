// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, FrameGeometry, HostPorts},
    clipboard::SystemClipboard,
    layout::Rect,
    terminal::TerminalOutput,
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
fn fixture() -> (TestRuntimeRoot, WorkspaceHost, ReadState) {
    let root = TestRuntimeRoot::new("context-reads").unwrap();
    let app = App::new_in_isolated_project(root.path(), HostPorts::isolated(Box::new(Clipboard)))
        .unwrap();
    (
        root,
        WorkspaceHost::new(app),
        ReadState::new("owner:1".into()),
    )
}
fn scopes() -> BTreeSet<Scope> {
    [
        Scope::TerminalRead,
        Scope::EditorContextRead,
        Scope::BufferEdit,
        Scope::TerminalPropose,
    ]
    .into()
}
fn call(host: &mut WorkspaceHost, state: &mut ReadState, request: Request) -> Value {
    host.context_read_request(state, &scopes(), request)
        .unwrap()
}
fn seed(host: &mut WorkspaceHost, text: &str) {
    let revision = host.app.buffers[0].revision();
    host.apply_expected_transaction(
        BufferId::from_index(0),
        BufferRevision::from_raw(revision),
        Transaction::insert(0, text),
    )
    .unwrap();
}
fn buffer(host: &mut WorkspaceHost, state: &mut ReadState) -> (String, String) {
    let list = call(
        host,
        state,
        Request::BufferList {
            offset: 0,
            limit: 10,
        },
    );
    (
        list["buffers"][0]["buffer"].as_str().unwrap().into(),
        list["buffers"][0]["revision"].as_str().unwrap().into(),
    )
}
fn pane(host: &mut WorkspaceHost, state: &mut ReadState) -> String {
    call(host, state, Request::PaneContextList(wire::Empty {}))["panes"][0]["pane"]
        .as_str()
        .unwrap()
        .into()
}
fn terminal(host: &mut WorkspaceHost, state: &mut ReadState) -> (TerminalId, String) {
    let id = host.app.terminals.insert_test_session(20, 3);
    host.app.terminals.apply(TerminalOutput::Bytes {
        id,
        bytes: b"one\r\ntwo\r\nthree".to_vec(),
    });
    let list = call(
        host,
        state,
        Request::TerminalList {
            offset: 0,
            limit: 10,
        },
    );
    (
        id,
        list["terminals"][0]["terminal"].as_str().unwrap().into(),
    )
}
fn read_terminal(terminal: String) -> Request {
    Request::TerminalRead {
        terminal,
        region: wire::Region::Tail,
        expected_revision: None,
        max_rows: 200,
        max_bytes: 65536,
        max_cells: 65536,
    }
}
fn viewport(pane: String) -> Request {
    Request::PaneViewportRead {
        pane,
        expected_revision: None,
        max_rows: 200,
        max_bytes: 65536,
        max_cells: 65536,
    }
}
fn capture(host: &mut WorkspaceHost) {
    let geometry = FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 22,
        },
        status: Rect {
            x: 0,
            y: 22,
            width: 80,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 23,
            width: 80,
            height: 1,
        },
    };
    let frame = host.prepare_frame(geometry);
    host.context.frame = Some(frame.editor);
    host.context.frame_id = frame.id.get();
    host.context.frame_foreground = host.app.plugins.foreground_generation;
    host.context.frame_attachment = host.app.plugins.attachment_generation;
    host.context.frame_review_sources = host
        .app
        .panes
        .iter()
        .filter_map(|(&id, pane)| {
            Some((
                id,
                host.app
                    .terminals
                    .get(pane.terminal?)?
                    .review_source_revision()?,
            ))
        })
        .collect();
    host.context.frame_sources = host
        .app
        .panes
        .iter()
        .map(|(&id, pane)| {
            (
                id,
                (
                    pane.buffer,
                    host.app.buffers[pane.buffer].revision(),
                    pane.terminal
                        .map(|id| (id, host.app.terminals.get(id).unwrap().read_revision())),
                ),
            )
        })
        .collect();
}

#[test]
fn scopes_and_connection_handles_are_checked_before_resource_access() {
    let (_root, mut host, mut state) = fixture();
    let (handle, revision) = buffer(&mut host, &mut state);
    assert_eq!(
        host.context_read_request(
            &mut state,
            &BTreeSet::new(),
            Request::BufferList {
                offset: 0,
                limit: 1
            }
        )
        .unwrap_err()
        .code,
        Code::CapabilityDenied
    );
    let mut other = ReadState::new("owner:2".into());
    assert_eq!(
        host.context_read_request(
            &mut other,
            &scopes(),
            Request::BufferRead {
                buffer: handle,
                expected_revision: revision,
                from: 0,
                to: 0
            }
        )
        .unwrap_err()
        .code,
        Code::NotFound
    );
    assert_eq!(
        host.context_read_request(
            &mut state,
            &[Scope::BufferEdit].into(),
            Request::BufferList {
                offset: 0,
                limit: 1
            }
        )
        .unwrap_err()
        .code,
        Code::CapabilityDenied
    );
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::TerminalInputStatus {
                proposal: "p:1".into()
            }
        )
        .unwrap_err()
        .code,
        Code::Unsupported
    );
}

#[test]
fn unsaved_unicode_reads_are_revision_checked_and_multiline_edits_undo_atomically() {
    let (_root, mut host, mut state) = fixture();
    seed(&mut host, "a界🙂z");
    let (handle, rev) = buffer(&mut host, &mut state);
    let focus = host.app.active_pane;
    assert_eq!(
        call(
            &mut host,
            &mut state,
            Request::BufferRead {
                buffer: handle.clone(),
                expected_revision: rev.clone(),
                from: 1,
                to: 3
            }
        )["text"],
        "界🙂"
    );
    let edit = Request::BufferEdit {
        buffer: handle.clone(),
        expected_revision: rev.clone(),
        changes: vec![
            editor::Change {
                from: 1,
                to: 2,
                text: "two\nlines".into(),
            },
            editor::Change {
                from: 3,
                to: 4,
                text: "!".into(),
            },
        ],
    };
    assert_eq!(
        host.context_read_request(&mut state, &[Scope::EditorContextRead].into(), edit.clone())
            .unwrap_err()
            .code,
        Code::CapabilityDenied
    );
    call(&mut host, &mut state, edit);
    assert!(host.app.plugins.presentation_dirty);
    assert_eq!(host.app.buffers[0].text().to_string(), "atwo\nlines🙂!");
    assert_eq!(host.app.active_pane, focus);
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::BufferRead {
                buffer: handle,
                expected_revision: rev,
                from: 0,
                to: 1
            }
        )
        .unwrap_err()
        .code,
        Code::Stale
    );
    assert!(host.app.buffers[0].undo());
    assert_eq!(host.app.buffers[0].text().to_string(), "a界🙂z");
}

#[test]
fn edits_reject_overlapping_ranges_without_partial_mutation() {
    let (_root, mut host, mut state) = fixture();
    seed(&mut host, "abcdef");
    let (handle, rev) = buffer(&mut host, &mut state);
    for changes in [
        vec![
            editor::Change {
                from: 0,
                to: 3,
                text: "x".into(),
            },
            editor::Change {
                from: 2,
                to: 4,
                text: "y".into(),
            },
        ],
        vec![editor::Change {
            from: 0,
            to: 99,
            text: "x".into(),
        }],
    ] {
        assert_eq!(
            host.context_read_request(
                &mut state,
                &scopes(),
                Request::BufferEdit {
                    buffer: handle.clone(),
                    expected_revision: rev.clone(),
                    changes
                }
            )
            .unwrap_err()
            .code,
            Code::InvalidArgument
        );
        assert_eq!(host.app.buffers[0].text().to_string(), "abcdef");
    }
}

#[test]
fn buffer_snapshots_survive_edits_expire_and_release_retention() {
    let (_root, mut host, mut state) = fixture();
    seed(&mut host, "old界");
    let (handle, rev) = buffer(&mut host, &mut state);
    let opened = call(
        &mut host,
        &mut state,
        Request::SnapshotOpen {
            buffer: handle.clone(),
            expected_revision: rev.clone(),
        },
    );
    let snapshot = opened["snapshot"].as_str().unwrap().to_owned();
    let charge = state.retained_bytes();
    assert!(charge >= 6);
    call(
        &mut host,
        &mut state,
        Request::BufferEdit {
            buffer: handle.clone(),
            expected_revision: rev,
            changes: vec![editor::Change {
                from: 0,
                to: 4,
                text: "new".into(),
            }],
        },
    );
    assert_eq!(
        call(
            &mut host,
            &mut state,
            Request::SnapshotRead {
                snapshot: snapshot.clone(),
                from: 0,
                to: 4
            }
        )["text"],
        "old界"
    );
    let mut other = ReadState::new("other".into());
    assert_eq!(
        host.context_read_request(
            &mut other,
            &scopes(),
            Request::SnapshotRead {
                snapshot: snapshot.clone(),
                from: 0,
                to: 1
            }
        )
        .unwrap_err()
        .code,
        Code::Closed
    );
    state.snapshots.get_mut(&snapshot).unwrap().last_read =
        Instant::now() - Duration::from_secs(31);
    state.expire();
    assert_eq!(state.retained_bytes(), 0);
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::SnapshotRead {
                snapshot,
                from: 0,
                to: 1
            }
        )
        .unwrap_err()
        .code,
        Code::Closed
    );
    let rev = revision(host.app.buffers[0].revision());
    for _ in 0..2 {
        call(
            &mut host,
            &mut state,
            Request::SnapshotOpen {
                buffer: handle.clone(),
                expected_revision: rev.clone(),
            },
        );
    }
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::SnapshotOpen {
                buffer: handle,
                expected_revision: rev
            }
        )
        .unwrap_err()
        .code,
        Code::LimitExceeded
    );
    let snapshot = state.snapshots.keys().next().unwrap().clone();
    call(&mut host, &mut state, Request::SnapshotClose { snapshot });
    assert_eq!(state.snapshots.len(), 1);
}

#[test]
fn terminal_reads_and_snapshots_are_inert_and_distinguish_stale_and_closed() {
    let (_root, mut host, mut state) = fixture();
    let (id, handle) = terminal(&mut host, &mut state);
    host.app.terminals.get_mut(id).unwrap().scroll_back(1);
    let before = host.app.terminals.get(id).unwrap().view(3);
    let result = call(&mut host, &mut state, read_terminal(handle.clone()));
    assert_eq!(result["rows"][0]["text"], "one");
    assert_eq!(host.app.terminals.get(id).unwrap().view(3), before);
    let opened = call(
        &mut host,
        &mut state,
        Request::TerminalSnapshotOpen {
            terminal: handle.clone(),
            region: wire::Region::Screen,
            expected_revision: Some(result["revision"].as_str().unwrap().into()),
            max_rows: 20,
            max_bytes: 65536,
            max_cells: 65536,
        },
    );
    let snapshot = opened["snapshot"].as_str().unwrap().to_owned();
    host.app.terminals.apply(TerminalOutput::Bytes {
        id,
        bytes: b"\rchanged".to_vec(),
    });
    let frozen = call(
        &mut host,
        &mut state,
        Request::TerminalSnapshotRead {
            snapshot: snapshot.clone(),
            offset: 0,
            limit: 1,
        },
    );
    assert_eq!(frozen["rows"][0]["text"], "one");
    assert_eq!(frozen["next"], 1);
    let mut stale = read_terminal(handle.clone());
    if let Request::TerminalRead {
        expected_revision, ..
    } = &mut stale
    {
        *expected_revision = Some(result["revision"].as_str().unwrap().into());
    }
    assert_eq!(
        host.context_read_request(&mut state, &scopes(), stale)
            .unwrap_err()
            .code,
        Code::Stale
    );
    host.app
        .terminals
        .apply(TerminalOutput::Exited { id, code: Some(0) });
    assert_eq!(
        call(&mut host, &mut state, read_terminal(handle.clone()))["live"],
        false
    );
    host.app.terminals.close(id);
    assert_eq!(
        host.context_read_request(&mut state, &scopes(), read_terminal(handle))
            .unwrap_err()
            .code,
        Code::Closed
    );
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::TerminalSnapshotRead {
                snapshot,
                offset: 0,
                limit: 1
            }
        )
        .unwrap_err()
        .code,
        Code::Closed
    );
    assert_eq!(state.retained_bytes(), 0);
}

#[test]
fn editor_read_alone_cannot_read_live_or_frozen_terminal_viewports() {
    let (_root, mut host, mut state) = fixture();
    let (id, _) = terminal(&mut host, &mut state);
    host.app
        .panes
        .get_mut(&host.app.active_pane)
        .unwrap()
        .terminal = Some(id);
    let handle = pane(&mut host, &mut state);
    let editor = [Scope::EditorContextRead].into();
    for review in [false, true] {
        if review {
            host.app.terminals.get_mut(id).unwrap().begin_review();
        }
        capture(&mut host);
        assert_eq!(
            host.context_read_request(&mut state, &editor, viewport(handle.clone()))
                .unwrap_err()
                .code,
            Code::CapabilityDenied
        );
        let result = call(&mut host, &mut state, viewport(handle.clone()));
        assert_eq!(result["rows"][0]["kind"], "terminal");
    }
    let inventory = host
        .context_read_request(
            &mut state,
            &editor,
            Request::PaneContextList(wire::Empty {}),
        )
        .unwrap();
    assert!(inventory["panes"][0]["terminal"].is_null());
    assert!(inventory["panes"][0]["buffer"].is_null());
}

#[test]
fn viewport_uses_only_actual_frame_and_rejects_detach_source_and_owner_changes() {
    let (_root, mut host, mut state) = fixture();
    seed(&mut host, "some text");
    let handle = pane(&mut host, &mut state);
    assert_eq!(
        host.context_read_request(&mut state, &scopes(), viewport(handle.clone()))
            .unwrap_err()
            .code,
        Code::NoFrontend
    );
    capture(&mut host);
    let value = call(&mut host, &mut state, viewport(handle.clone()));
    assert_eq!(value["rows"][0]["kind"], "text");
    host.app.plugins.foreground_generation += 1;
    assert_eq!(
        host.context_read_request(&mut state, &scopes(), viewport(handle.clone()))
            .unwrap_err()
            .code,
        Code::ContextChanged
    );
    capture(&mut host);
    host.app.plugins.attachment_generation += 1;
    assert_eq!(
        host.context_read_request(&mut state, &scopes(), viewport(handle.clone()))
            .unwrap_err()
            .code,
        Code::ContextChanged
    );
    capture(&mut host);
    seed(&mut host, "changed");
    assert_eq!(
        host.context_read_request(&mut state, &scopes(), viewport(handle))
            .unwrap_err()
            .code,
        Code::Stale
    );
}

#[test]
fn pane_retarget_and_close_invalidate_owned_handles_and_selection_source() {
    let (_root, mut host, mut state) = fixture();
    seed(&mut host, "text");
    let handle = pane(&mut host, &mut state);
    assert!(
        call(
            &mut host,
            &mut state,
            Request::SelectionGet {
                pane: handle.clone()
            }
        )["ranges"]
            .is_array()
    );
    let id = host.app.terminals.insert_test_session(10, 2);
    let paneid = host.app.active_pane;
    host.app.panes.get_mut(&paneid).unwrap().terminal = Some(id);
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::SelectionGet { pane: handle }
        )
        .unwrap_err()
        .code,
        Code::Stale
    );
    let handle = pane(&mut host, &mut state);
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::SelectionGet {
                pane: handle.clone()
            }
        )
        .unwrap_err()
        .code,
        Code::Conflict
    );
    host.app.panes.remove(&paneid);
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::SelectionGet { pane: handle }
        )
        .unwrap_err()
        .code,
        Code::Closed
    );
}

#[test]
fn snapshot_and_handle_budgets_are_per_reader_and_charge_allocations() {
    let mut state = ReadState::new("budget".into());
    for index in 0..MAX_HANDLES {
        state.buffer(index).unwrap();
    }
    assert_eq!(
        state.buffer(MAX_HANDLES).unwrap_err().code,
        Code::LimitExceeded
    );
    assert!(state.buffer(0).is_ok());
    let mut state = ReadState::new("snapshot".into());
    let capture = Capture::Buffer {
        index: 0,
        handle: "b".into(),
        revision: "r".into(),
        text: String::new(),
        checkpoints: Vec::new(),
        chars: 0,
    };
    assert_eq!(
        state
            .store(capture, wire::MAX_RETAINED_BYTES + 1)
            .unwrap_err()
            .code,
        Code::LimitExceeded
    );
}

#[test]
fn generated_buffers_are_readable_but_not_editable_and_ranges_remain_bounded() {
    let (_root, mut host, mut state) = fixture();
    host.app
        .buffers
        .push(crate::buffer::Buffer::help("reference"));
    let index = host.app.buffers.len() - 1;
    let handle = state.buffer(index).unwrap();
    let expected_revision = revision(host.app.buffers[index].revision());
    assert_eq!(
        call(
            &mut host,
            &mut state,
            Request::BufferRead {
                buffer: handle.clone(),
                expected_revision: expected_revision.clone(),
                from: 0,
                to: 9,
            }
        )["text"],
        "reference"
    );
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::BufferEdit {
                buffer: handle.clone(),
                expected_revision: expected_revision.clone(),
                changes: vec![editor::Change {
                    from: 0,
                    to: 0,
                    text: "change".into()
                }],
            }
        )
        .unwrap_err()
        .code,
        Code::ReadOnly
    );
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::BufferRead {
                buffer: handle,
                expected_revision,
                from: 0,
                to: 99,
            }
        )
        .unwrap_err()
        .code,
        Code::InvalidArgument
    );
}

#[test]
fn terminal_snapshot_types_offsets_and_closure_are_explicit() {
    let (_root, mut host, mut state) = fixture();
    let (_, terminal) = terminal(&mut host, &mut state);
    let opened = call(
        &mut host,
        &mut state,
        Request::TerminalSnapshotOpen {
            terminal,
            region: wire::Region::Tail,
            expected_revision: None,
            max_rows: 20,
            max_bytes: 65536,
            max_cells: 65536,
        },
    );
    let snapshot = opened["snapshot"].as_str().unwrap().to_owned();
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::SnapshotRead {
                snapshot: snapshot.clone(),
                from: 0,
                to: 1,
            }
        )
        .unwrap_err()
        .code,
        Code::InvalidArgument
    );
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::TerminalSnapshotRead {
                snapshot: snapshot.clone(),
                offset: 100,
                limit: 1,
            }
        )
        .unwrap_err()
        .code,
        Code::InvalidArgument
    );
    call(
        &mut host,
        &mut state,
        Request::TerminalSnapshotClose {
            snapshot: snapshot.clone(),
        },
    );
    assert_eq!(state.retained_bytes(), 0);
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::TerminalSnapshotClose { snapshot }
        )
        .unwrap_err()
        .code,
        Code::Closed
    );
}

#[test]
fn snapshots_share_terminal_history_budget_without_evicting_live_state() {
    let (_root, mut host, mut state) = fixture();
    let id = host.app.terminals.insert_test_session(1, 2);
    let handle = state.terminal_handle(id).unwrap();
    let cell_bytes = std::mem::size_of::<crate::terminal::Cell>();
    host.app.terminals.set_memory_budget_for_test(1);
    host.app.terminals.apply(TerminalOutput::Bytes {
        id,
        bytes: b"a\r\nb\r\nc".to_vec(),
    });
    assert_eq!(host.app.terminals.retained_payload_bytes(), cell_bytes);
    let before = host.app.terminals.get(id).unwrap().view(3);
    let capture = || Request::TerminalSnapshotOpen {
        terminal: handle.clone(),
        region: wire::Region::Tail,
        expected_revision: None,
        max_rows: 20,
        max_bytes: 65536,
        max_cells: 65536,
    };
    assert_eq!(
        host.context_read_request(&mut state, &scopes(), capture())
            .unwrap_err()
            .code,
        Code::LimitExceeded
    );
    assert_eq!(host.app.terminals.get(id).unwrap().view(3), before);
    host.app.terminals.set_memory_budget_for_test(200);
    let opened = call(&mut host, &mut state, capture());
    let charge = state.retained_bytes();
    assert!(charge > 0);
    host.app.terminals.set_external_retained_bytes(charge);
    host.app
        .terminals
        .set_memory_budget_for_test(charge.div_ceil(cell_bytes) + 1);
    host.app.terminals.apply(TerminalOutput::Bytes {
        id,
        bytes: b"\r\nd\r\ne\r\nf".to_vec(),
    });
    assert!(
        host.app.terminals.retained_payload_bytes() + charge
            <= host.app.terminals.retained_payload_limit()
    );
    call(
        &mut host,
        &mut state,
        Request::TerminalSnapshotClose {
            snapshot: opened["snapshot"].as_str().unwrap().into(),
        },
    );
    host.app
        .terminals
        .set_external_retained_bytes(state.retained_bytes());
    let before = host.app.terminals.retained_payload_bytes();
    host.app.terminals.apply(TerminalOutput::Bytes {
        id,
        bytes: b"\r\ng".to_vec(),
    });
    assert!(host.app.terminals.retained_payload_bytes() > before);
}

#[test]
fn pane_a_to_b_to_a_never_revalidates_the_original_handle() {
    let (root, mut host, mut state) = fixture();
    let a = root.path().join("a.txt");
    let b = root.path().join("b.txt");
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    host.open_buffer(a.clone(), true).unwrap();
    let original = pane(&mut host, &mut state);
    host.open_buffer(b, true).unwrap();
    host.open_buffer(a, true).unwrap();
    assert_eq!(
        host.context_read_request(
            &mut state,
            &scopes(),
            Request::SelectionGet {
                pane: original.clone()
            }
        )
        .unwrap_err()
        .code,
        Code::Stale
    );
    let current = pane(&mut host, &mut state);
    assert_ne!(current, original);
    assert!(
        call(
            &mut host,
            &mut state,
            Request::SelectionGet { pane: current }
        )["ranges"]
            .is_array()
    );
}

#[test]
fn context_edit_does_not_save_or_complete_external_editor_wait() {
    let (root, mut host, mut state) = fixture();
    let path = root.path().join("prompt.txt");
    std::fs::write(&path, "original").unwrap();
    let (token, buffers) = host.create_wait_request([path.clone()], false).unwrap();
    let index = buffers[0].index().unwrap();
    let handle = state.buffer(index).unwrap();
    let rev = revision(host.app.buffers[index].revision());
    let pane_before = host.app.panes[&host.app.active_pane].buffer;
    call(
        &mut host,
        &mut state,
        Request::BufferEdit {
            buffer: handle,
            expected_revision: rev,
            changes: vec![editor::Change {
                from: 0,
                to: 8,
                text: "agent\nedit".into(),
            }],
        },
    );
    host.reconcile_wait_requests();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    assert!(matches!(
        host.wait_status(token),
        Some(crate::workspace::WaitStatus::Pending { .. })
    ));
    assert_eq!(host.app.panes[&host.app.active_pane].buffer, pane_before);
    assert_eq!(host.app.buffers[index].text().to_string(), "agent\nedit");
}

#[test]
fn late_unicode_snapshot_ranges_use_bounded_sparse_offsets() {
    let (_root, mut host, mut state) = fixture();
    let text = "界🙂x".repeat(150000);
    seed(&mut host, &text);
    let (handle, rev) = buffer(&mut host, &mut state);
    let result = call(
        &mut host,
        &mut state,
        Request::SnapshotOpen {
            buffer: handle,
            expected_revision: rev,
        },
    );
    let snapshot = result["snapshot"].as_str().unwrap().to_owned();
    assert_eq!(
        call(
            &mut host,
            &mut state,
            Request::SnapshotRead {
                snapshot: snapshot.clone(),
                from: 449997,
                to: 450000
            }
        )["text"],
        "界🙂x"
    );
    let Capture::Buffer {
        text,
        checkpoints,
        chars,
        ..
    } = &state.snapshots[&snapshot].capture
    else {
        panic!("buffer snapshot")
    };
    assert_eq!(checkpoints.len(), chars.div_ceil(256));
    assert_eq!(snapshot_byte(text, checkpoints, *chars, *chars), text.len());
    assert_eq!(
        call(
            &mut host,
            &mut state,
            Request::SnapshotRead {
                snapshot,
                from: 450000,
                to: 450000
            }
        )["text"],
        ""
    );
}
