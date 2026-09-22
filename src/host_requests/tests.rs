// SPDX-License-Identifier: MPL-2.0

use super::*;
use runyte::{
    app::App,
    config::Config,
    external_open::ProgramCache,
    protocol::{WaitStatus, WaitToken},
    test_support::TestRuntimeRoot,
};

fn fixture(label: &str) -> (TestRuntimeRoot, WorkspaceHost) {
    let root = TestRuntimeRoot::new(label).unwrap();
    let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
    app.note_loaded_config(&root.join("config.yaml"));
    app.programs = ProgramCache::load(Some(root.join("program-cache")));
    (root, WorkspaceHost::new(app))
}

#[test]
fn read_only_wait_responses_do_not_publish_editor_frames() {
    let token: WaitToken = serde_json::from_str("1").unwrap();
    let response = HostResponse::WaitState {
        token,
        status: WaitStatus::Pending {
            buffers: Vec::new(),
            remaining: Vec::new(),
        },
        interactive_attached: true,
    };

    assert!(!workspace_response_publishes_frame(&response));
}

#[test]
fn destination_inventory_is_identity_only_and_visits_require_current_host_and_interactive_role() {
    let (_root, mut host) = fixture("host-destinations");
    let current = host.active().buffer;
    let response = super::handle_workspace_request(
        &mut host,
        runyte::protocol::ClientRequest::DestinationInventory,
        false,
        false,
    )
    .unwrap()
    .response;
    let HostResponse::DestinationInventory {
        incarnation,
        entries,
        truncated,
    } = response
    else {
        panic!("expected inventory")
    };
    assert!(!truncated);
    assert!(!entries.is_empty());
    let destination = entries[0].destination;
    let response = super::handle_workspace_request(
        &mut host,
        runyte::protocol::ClientRequest::VisitDestination {
            incarnation: incarnation.clone(),
            destination,
        },
        false,
        false,
    )
    .unwrap()
    .response;
    assert!(matches!(response, HostResponse::Error { .. }));
    let response = super::handle_workspace_request(
        &mut host,
        runyte::protocol::ClientRequest::VisitDestination {
            incarnation: "0".repeat(64),
            destination,
        },
        true,
        true,
    )
    .unwrap()
    .response;
    assert!(matches!(
        response,
        HostResponse::DestinationVisitResult { error: Some(_) }
    ));
    assert_eq!(host.active().buffer, current);
    let response = super::handle_workspace_request(
        &mut host,
        runyte::protocol::ClientRequest::VisitDestination {
            incarnation: incarnation.clone(),
            destination: runyte::protocol::OpenDestination::Buffer(u64::MAX),
        },
        true,
        true,
    )
    .unwrap()
    .response;
    assert!(matches!(
        response,
        HostResponse::DestinationVisitResult { error: Some(_) }
    ));
    let response = super::handle_workspace_request(
        &mut host,
        runyte::protocol::ClientRequest::VisitDestination {
            incarnation,
            destination,
        },
        true,
        true,
    )
    .unwrap()
    .response;
    assert!(matches!(
        response,
        HostResponse::DestinationVisitResult { error: None }
    ));
}

fn control(host: &mut WorkspaceHost, request: ClientRequest) -> WorkspaceReply {
    assert!(is_workspace_request(&request));
    handle_workspace_request(host, request, false, false).unwrap()
}

#[test]
fn health_is_read_only_and_lifecycle_requests_stay_with_the_host_loop() {
    let (_root, mut host) = fixture("host-health");
    assert!(!is_workspace_request(&ClientRequest::Shutdown));
    assert!(handle_workspace_request(&mut host, ClientRequest::Shutdown, false, false).is_none());
    let reply = control(&mut host, ClientRequest::Health);
    assert!(!reply.publish_frame);
    assert!(matches!(reply.response, HostResponse::Health {
        protocol: runyte::protocol::VERSION,
        pid,
        interactive_attached: false,
        pending_wait_requests: 0,
        live_terminals: 0,
        ..
    } if pid == std::process::id()));
    let reply = handle_workspace_request(&mut host, ClientRequest::Health, true, false).unwrap();
    assert!(matches!(
        reply.response,
        HostResponse::Health {
            interactive_attached: true,
            ..
        }
    ));
}

#[test]
fn destination_labels_respect_the_byte_budget_without_splitting_utf8() {
    let limit = runyte::protocol::MAX_DESTINATION_LABEL_BYTES;
    let prefix = "a".repeat(limit - 1);
    assert_eq!(bounded_destination_label(&format!("{prefix}😀")), prefix);
    let value = "é".repeat(limit);
    let bounded = bounded_destination_label(&value);
    assert_eq!(bounded.len(), limit);
    assert_eq!(bounded, "é".repeat(limit / 2));
    assert_eq!(bounded_destination_label("small 文"), "small 文");
}

#[test]
fn destination_inventory_bounds_large_working_sets_and_marks_truncation() {
    let (_root, mut host) = fixture("host-inventory-bound");
    for _ in 0..=runyte::protocol::MAX_DESTINATIONS {
        let mut buffer = runyte::buffer::Buffer::scratch();
        buffer.apply(&runyte::text::Transaction::insert(0, "visible"));
        host.app_mut().buffers.push(buffer);
    }
    let reply = control(&mut host, ClientRequest::DestinationInventory);
    assert!(!reply.publish_frame);
    let HostResponse::DestinationInventory {
        entries, truncated, ..
    } = reply.response
    else {
        panic!("expected destination inventory");
    };
    assert!(truncated);
    assert_eq!(entries.len(), runyte::protocol::MAX_DESTINATIONS);
    assert!(
        entries
            .iter()
            .all(|entry| entry.label.len() <= runyte::protocol::MAX_DESTINATION_LABEL_BYTES)
    );
}

#[test]
fn revision_checked_mutations_publish_frames_but_stale_or_unauthorized_requests_do_not() {
    let (root, mut host) = fixture("host-revisions");
    let path = root.join("document.txt");
    std::fs::write(&path, "before").unwrap();
    let opened = control(
        &mut host,
        ClientRequest::OpenBuffers {
            paths: vec![runyte::protocol::encode_path(&path)],
            activate: true,
        },
    );
    assert!(opened.publish_frame);
    let HostResponse::Opened { buffers } = opened.response else {
        panic!("expected opened buffer");
    };
    let buffer = buffers[0];
    let expected = host
        .read_buffer(buffer.into())
        .unwrap()
        .metadata
        .revision
        .into();
    let request = ClientRequest::ApplyTransaction {
        buffer,
        expected,
        changes: vec![TransportChange {
            from: 0,
            to: 6,
            text: "after".into(),
        }],
    };
    let applied = control(&mut host, request.clone());
    assert!(applied.publish_frame);
    assert!(matches!(
        applied.response,
        HostResponse::TransactionApplied { .. }
    ));
    let stale = control(&mut host, request);
    assert!(!stale.publish_frame);
    assert!(matches!(stale.response, HostResponse::StaleRevision { .. }));
    let refused = control(
        &mut host,
        ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest::at(
                "select-all",
                serde_json::from_str("1").unwrap(),
                buffer,
                expected,
            ),
        },
    );
    assert!(!refused.publish_frame);
    assert!(
        matches!(refused.response, HostResponse::Error { message } if message == "semantic commands require the attached interactive client")
    );
    assert_eq!(host.read_buffer(buffer.into()).unwrap().text, "after");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "before");
}

#[test]
fn wait_creation_and_completion_preserve_frame_and_attachment_contracts() {
    let (root, mut host) = fixture("host-wait");
    let path = root.join("wait.txt");
    std::fs::write(&path, "clean").unwrap();
    let created = control(
        &mut host,
        ClientRequest::CreateWait {
            paths: vec![runyte::protocol::encode_path(&path)],
        },
    );
    assert!(created.publish_frame);
    let HostResponse::WaitCreated {
        token,
        buffers,
        interactive_attached,
    } = created.response
    else {
        panic!("expected wait token");
    };
    assert!(!interactive_attached);
    assert_eq!(buffers.len(), 1);
    let pending = control(&mut host, ClientRequest::WaitStatus { token });
    assert!(!pending.publish_frame);
    assert!(matches!(
        pending.response,
        HostResponse::WaitState {
            status: WaitStatus::Pending { .. },
            ..
        }
    ));
    let completed = control(
        &mut host,
        ClientRequest::CompleteWaitBuffer {
            token,
            buffer: buffers[0],
        },
    );
    assert!(!completed.publish_frame);
    assert!(matches!(
        completed.response,
        HostResponse::WaitState {
            status: WaitStatus::Completed,
            ..
        }
    ));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "clean");
}

#[cfg(windows)]
#[test]
fn malformed_native_paths_return_request_errors_without_partial_buffer_or_wait_changes() {
    let (root, mut host) = fixture("host-bad-path");
    let path = root.join("valid.txt");
    std::fs::write(&path, "valid").unwrap();
    let paths = vec![runyte::protocol::encode_path(&path), vec![0x41]];
    let before = host.open_buffer_count();
    for request in [
        ClientRequest::OpenBuffers {
            paths: paths.clone(),
            activate: true,
        },
        ClientRequest::CreateWait { paths },
    ] {
        let reply = control(&mut host, request);
        assert!(matches!(reply.response, HostResponse::Error { .. }));
        assert!(!reply.publish_frame);
        assert_eq!(host.open_buffer_count(), before);
        assert_eq!(host.protected_state().pending_wait_requests, 0);
    }
    assert!(matches!(
        control(&mut host, ClientRequest::Health).response,
        HostResponse::Health { .. }
    ));
}

#[cfg(unix)]
#[test]
fn unix_native_path_bytes_still_open_non_utf8_filenames() {
    use std::os::unix::ffi::OsStringExt;
    let (root, mut host) = fixture("host-unix-path");
    let path = root.join(std::ffi::OsString::from_vec(b"native-\xff.txt".to_vec()));
    std::fs::write(&path, "raw path").unwrap();
    let reply = control(
        &mut host,
        ClientRequest::OpenBuffers {
            paths: vec![runyte::protocol::encode_path(&path)],
            activate: true,
        },
    );
    assert!(reply.publish_frame);
    let HostResponse::Opened { buffers } = reply.response else {
        panic!("expected opened native path");
    };
    assert_eq!(
        host.read_buffer(buffers[0].into()).unwrap().text,
        "raw path"
    );
}
