// SPDX-License-Identifier: MPL-2.0

use super::*;
use futures_util::{FutureExt, StreamExt};
use runyte::{
    app::App,
    config::Config,
    external_open::ProgramCache,
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{EndpointLocation, RegistrySet},
        windows_transport::{ResponseReceiver, response_channel},
    },
};

struct Fixture {
    host: WorkspaceHost,
    metadata: EndpointMetadata,
    proof: Arc<PinnedProcess>,
    root: TestRuntimeRoot,
}

impl Fixture {
    fn new() -> Self {
        let root = TestRuntimeRoot::new("native-host-clients").unwrap();
        let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
        app.note_loaded_config(&root.join("config.yaml"));
        app.programs = ProgramCache::load(Some(root.join("program-cache")));
        let location = EndpointLocation::new(
            root.path(),
            root.join("endpoint"),
            RegistrySet::open(&[root.join("registry")]).unwrap(),
        )
        .unwrap();
        let prepared = location.prepare(Some("initial".into())).unwrap();
        let metadata = prepared.metadata().clone();
        drop(prepared);
        let proof = Arc::new(PinnedProcess::open_peer(std::process::id()).unwrap());
        Self {
            host: WorkspaceHost::new(app),
            metadata,
            proof,
            root,
        }
    }

    fn connect(&mut self, clients: &mut Clients, id: u64) -> ResponseReceiver {
        self.connect_with_role(clients, id, false)
    }

    fn connect_interactive(&mut self, clients: &mut Clients, id: u64) -> ResponseReceiver {
        self.connect_with_role(clients, id, true)
    }

    fn connect_with_role(
        &mut self,
        clients: &mut Clients,
        id: u64,
        interactive: bool,
    ) -> ResponseReceiver {
        let (tx, rx) = response_channel();
        clients.connected(
            &mut self.host,
            ConnectedPeer {
                id,
                proof: self.proof.clone(),
                responses: tx,
                interactive,
                geometry: runyte::app::FrameGeometry::default(),
            },
        );
        assert!(clients.peers.contains_key(&id));
        rx
    }

    fn wait(&mut self, clients: &mut Clients, id: u64, name: &str) -> WaitToken {
        let path = self.root.join(name);
        std::fs::write(&path, "clean").unwrap();
        request(
            clients,
            &mut self.host,
            id,
            ClientRequest::CreateWait {
                paths: vec![runyte::protocol::encode_path(&path)],
            },
        );
        *clients.peers[&id].waits.iter().next().unwrap()
    }
}

#[test]
fn native_switch_abort_preserves_waits_and_commit_cancels_only_source_owned_waits() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _interactive = fixture.connect_interactive(&mut clients, 1);
    let _control = fixture.connect(&mut clients, 2);
    let source_wait = fixture.wait(&mut clients, 1, "source-wait.txt");
    let control_wait = fixture.wait(&mut clients, 2, "control-wait.txt");

    assert!(clients.reserve_switch(&mut fixture.host, 1, 10));
    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::NativeSwitchAbort { receipt: 10 },
    );
    assert_eq!(
        clients.take_switch_action(),
        Some(NativeSwitchAction::Abort {
            owner: 1,
            receipt: 10
        })
    );
    assert!(clients.abort_switch(&mut fixture.host, 1, 10));
    for token in [source_wait, control_wait] {
        assert!(matches!(
            fixture.host.wait_status(token.into()),
            Some(runyte::workspace::WaitStatus::Pending { .. })
        ));
    }

    assert!(clients.reserve_switch(&mut fixture.host, 1, 11));
    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::NativeSwitchCommit { receipt: 11 },
    );
    assert_eq!(
        clients.take_switch_action(),
        Some(NativeSwitchAction::Commit {
            owner: 1,
            receipt: 11
        })
    );
    assert!(clients.commit_switch(&mut fixture.host, 1, 11));
    assert!(matches!(
        fixture.host.wait_status(source_wait.into()),
        Some(runyte::workspace::WaitStatus::Cancelled { .. })
    ));
    assert!(matches!(
        fixture.host.wait_status(control_wait.into()),
        Some(runyte::workspace::WaitStatus::Pending { .. })
    ));
    assert_eq!(clients.active_id(), None);
}

#[test]
fn late_native_switch_receipt_cannot_release_a_replacement_owner() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _old = fixture.connect_interactive(&mut clients, 1);
    assert!(clients.reserve_switch(&mut fixture.host, 1, 22));
    clients.disconnected(&mut fixture.host, 1);
    let _replacement = fixture.connect_interactive(&mut clients, 2);

    request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::NativeSwitchCommit { receipt: 22 },
    );
    assert!(clients.take_switch_action().is_none());
    assert_eq!(clients.active_id(), Some(2));
}

#[test]
fn prepared_response_send_failure_releases_reservation_and_owner() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let receiver = fixture.connect_interactive(&mut clients, 7);
    drop(receiver);
    assert!(clients.reserve_switch(&mut fixture.host, 7, 31));
    let metadata = &fixture.metadata;
    let candidate = runyte::protocol::NativeSwitchCandidate {
        protocol: metadata.protocol,
        id: metadata.id.clone(),
        name: metadata.name.clone(),
        project_root_bytes: metadata.project_root_bytes.clone(),
        process_pid: metadata.process.pid,
        process_creation_time: metadata.process.creation_time,
        incarnation: metadata.incarnation.clone(),
        address: metadata.address.as_str().to_owned(),
        publication_key: runyte::workspace::PublicationKey::from_authenticated_metadata(metadata)
            .to_bytes(),
    };
    assert!(!clients.begin_switch(
        &mut fixture.host,
        7,
        31,
        HostResponse::NativeSwitchPrepared {
            receipt: 31,
            candidate: Box::new(candidate),
        },
    ));
    assert_eq!(clients.active_id(), None);
    assert!(!clients.switch_pending());
    assert!(!clients.peers.contains_key(&7));
}

fn no_rename(_: &str) -> io::Result<RenameFuture> {
    panic!("unexpected rename admission")
}

fn request(
    clients: &mut Clients,
    host: &mut WorkspaceHost,
    id: u64,
    request: ClientRequest,
) -> bool {
    clients.incoming(host, id, Incoming::Request(request), no_rename)
}

#[test]
fn held_rename_defers_one_request_while_another_peer_progresses() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _first = fixture.connect(&mut clients, 1);
    let _second = fixture.connect(&mut clients, 2);
    let (complete, result) = tokio::sync::oneshot::channel();
    clients.incoming(
        &mut fixture.host,
        1,
        Incoming::Request(ClientRequest::RenameHost {
            name: "first".into(),
        }),
        |name| {
            assert_eq!(name, "first");
            Ok(Box::pin(async move { result.await.unwrap() }))
        },
    );
    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::RenameHost {
            name: "second".into(),
        },
    );
    let other_wait = fixture.wait(&mut clients, 2, "other.txt");
    assert!(matches!(
        fixture.host.wait_status(other_wait.into()),
        Some(runyte::workspace::WaitStatus::Pending { .. })
    ));
    assert!(clients.renames.next().now_or_never().is_none());
    let mut metadata = fixture.metadata.clone();
    metadata.name = Some("first".into());
    complete.send(Ok(metadata.clone())).unwrap();
    let (id, result) = clients.renames.next().now_or_never().unwrap().unwrap();
    let mut admitted_second = false;
    clients.renamed(&mut fixture.host, id, result, |name| {
        assert_eq!(name, "second");
        admitted_second = true;
        metadata.name = Some("second".into());
        Ok(Box::pin(std::future::ready(Ok(metadata))))
    });
    assert!(admitted_second);
    assert!(clients.peers[&1].renaming);
    request(&mut clients, &mut fixture.host, 1, ClientRequest::Health);
    let (id, result) = clients.renames.next().now_or_never().unwrap().unwrap();
    clients.renamed(&mut fixture.host, id, result, no_rename);
    assert!(!clients.peers[&1].renaming);
    assert!(clients.peers[&1].deferred.is_none());
}

#[test]
fn rename_failure_dispatches_deferred_protocol_error_without_reordering_shutdown() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _receiver = fixture.connect(&mut clients, 1);
    clients.incoming(
        &mut fixture.host,
        1,
        Incoming::Request(ClientRequest::RenameHost {
            name: "first".into(),
        }),
        |_| Ok(Box::pin(std::future::pending())),
    );
    clients.incoming(
        &mut fixture.host,
        1,
        Incoming::ProtocolError("role refused".into()),
        no_rename,
    );
    assert!(matches!(
        clients.peers[&1].deferred,
        Some(Incoming::ProtocolError(_))
    ));
    assert!(!clients.renamed(
        &mut fixture.host,
        1,
        Err(io::Error::other("rename refused")),
        no_rename
    ));
    assert!(clients.peers[&1].deferred.is_none());
    assert!(request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::Shutdown
    ));
}

#[test]
fn rename_overflow_drops_only_its_wait_owner_and_ignores_late_completion() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _first = fixture.connect(&mut clients, 1);
    let _second = fixture.connect(&mut clients, 2);
    let first = fixture.wait(&mut clients, 1, "shared.txt");
    let second = fixture.wait(&mut clients, 2, "shared.txt");
    clients.incoming(
        &mut fixture.host,
        1,
        Incoming::Request(ClientRequest::RenameHost {
            name: "first".into(),
        }),
        |_| Ok(Box::pin(std::future::pending())),
    );
    request(&mut clients, &mut fixture.host, 1, ClientRequest::Health);
    clients.incoming(
        &mut fixture.host,
        1,
        Incoming::ProtocolError("later".into()),
        no_rename,
    );
    assert!(!clients.peers.contains_key(&1));
    assert!(matches!(
        fixture.host.wait_status(first.into()),
        Some(runyte::workspace::WaitStatus::Cancelled { .. })
    ));
    assert!(matches!(
        fixture.host.wait_status(second.into()),
        Some(runyte::workspace::WaitStatus::Pending { .. })
    ));
    assert!(!clients.renamed(
        &mut fixture.host,
        1,
        Ok(fixture.metadata.clone()),
        no_rename
    ));
    assert!(!request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::ForceShutdown
    ));
    fixture.host.complete_wait_request(second.into()).unwrap();
    clients.disconnected(&mut fixture.host, 2);
    assert!(matches!(
        fixture.host.wait_status(second.into()),
        Some(runyte::workspace::WaitStatus::Completed)
    ));
}

#[test]
fn response_backpressure_cancels_pending_wait_without_changing_completed_wait() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _receiver = fixture.connect(&mut clients, 1);
    let completed = fixture.wait(&mut clients, 1, "completed.txt");
    fixture
        .host
        .complete_wait_request(completed.into())
        .unwrap();
    clients.reconcile(&mut fixture.host);
    let pending = fixture.wait(&mut clients, 1, "pending.txt");
    for _ in 0..1024 {
        if !clients.peers.contains_key(&1) {
            break;
        }
        request(&mut clients, &mut fixture.host, 1, ClientRequest::Health);
    }
    assert!(!clients.peers.contains_key(&1));
    assert!(matches!(
        fixture.host.wait_status(completed.into()),
        Some(runyte::workspace::WaitStatus::Completed)
    ));
    assert!(matches!(
        fixture.host.wait_status(pending.into()),
        Some(runyte::workspace::WaitStatus::Cancelled { .. })
    ));
}

#[test]
fn interactive_peer_is_unique_and_pending_control_wait_protects_shutdown() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let (tx, _rx) = response_channel();
    clients.connected(
        &mut fixture.host,
        ConnectedPeer {
            id: 1,
            proof: fixture.proof.clone(),
            responses: tx,
            interactive: true,
            geometry: runyte::app::FrameGeometry::default(),
        },
    );
    assert_eq!(clients.active, Some(1));
    let (refused, _response) = response_channel();
    clients.connected(
        &mut fixture.host,
        ConnectedPeer {
            id: 3,
            proof: fixture.proof.clone(),
            responses: refused,
            interactive: true,
            geometry: runyte::app::FrameGeometry::default(),
        },
    );
    assert_eq!(clients.active, Some(1));
    assert!(!clients.peers.contains_key(&3));
    let _receiver = fixture.connect(&mut clients, 2);
    fixture.wait(&mut clients, 2, "protected.txt");
    assert!(!request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::Shutdown
    ));
    assert!(request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::ForceShutdown
    ));
}

#[test]
fn stale_interactive_connection_id_cannot_redirect_input_or_clear_replacement() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let (first, _first_receiver) = response_channel();
    clients.connected(
        &mut fixture.host,
        ConnectedPeer {
            id: 1,
            proof: fixture.proof.clone(),
            responses: first,
            interactive: true,
            geometry: runyte::app::FrameGeometry::default(),
        },
    );
    let first_wait = fixture.wait(&mut clients, 1, "old-id-wait.txt");
    clients.disconnected(&mut fixture.host, 1);
    assert!(matches!(
        fixture.host.wait_status(first_wait.into()),
        Some(runyte::workspace::WaitStatus::Cancelled { .. })
    ));
    let (replacement, _replacement_receiver) = response_channel();
    clients.connected(
        &mut fixture.host,
        ConnectedPeer {
            id: 2,
            proof: fixture.proof.clone(),
            responses: replacement,
            interactive: true,
            geometry: runyte::app::FrameGeometry::default(),
        },
    );
    let second_wait = fixture.wait(&mut clients, 2, "new-id-wait.txt");
    let active_buffer = fixture.host.app().active().buffer;
    let before_mode = fixture.host.app().mode;
    let before_revision = fixture.host.app().buffers[active_buffer].revision();
    assert!(!request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::Input {
            event: runyte::input::InputEvent::Key(runyte::input::KeyStroke::char(':')).into(),
            repeated: false,
            presented_frame: None,
        }
    ));
    assert!(!request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::ForceShutdown
    ));
    clients.disconnected(&mut fixture.host, 1);
    assert_eq!(clients.active, Some(2));
    assert_eq!(fixture.host.app().mode, before_mode);
    assert_eq!(
        fixture.host.app().buffers[active_buffer].revision(),
        before_revision
    );
    assert!(clients.peers.contains_key(&2));
    assert!(matches!(
        fixture.host.wait_status(second_wait.into()),
        Some(runyte::workspace::WaitStatus::Pending { .. })
    ));
}
