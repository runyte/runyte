// SPDX-License-Identifier: MPL-2.0

use super::*;
use futures_util::{FutureExt, StreamExt};
use runyte::{
    app::App,
    config::Config,
    external_open::ProgramCache,
    terminal::TerminalRequest,
    test_support::TestRuntimeRoot,
    workspace::{
        parent::{ParentContext, ParentLaunch},
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

    fn acknowledge_initial_frame(&mut self, clients: &mut Clients, id: u64) {
        let frame = clients.peers[&id]
            .pending_ready_frames
            .expect("interactive connection was issued a complete frame");
        request(
            clients,
            &mut self.host,
            id,
            ClientRequest::FrameDrawn { frame: frame.1 },
        );
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
        let before = clients.peers[&id].waits.clone();
        request(
            clients,
            &mut self.host,
            id,
            ClientRequest::CreateWait {
                paths: vec![runyte::protocol::encode_path(&path)],
            },
        );
        let created: Vec<_> = clients.peers[&id]
            .waits
            .difference(&before)
            .copied()
            .collect();
        assert_eq!(
            created.len(),
            1,
            "CreateWait must add exactly one owned token"
        );
        created[0]
    }

    fn show_terminal(&mut self, terminal: runyte::terminal::TerminalId) {
        let command =
            runyte::command::parse_colon_command(&format!("terminal-show {terminal}")).unwrap();
        self.host.app_mut().execute(command).unwrap();
        assert_eq!(self.host.app().active_terminal(), Some(terminal));
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
fn parent_switch_requires_original_frontend_to_confirm_nonfinal_commit_ack() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _source = fixture.connect_interactive(&mut clients, 1);
    let _control = fixture.connect(&mut clients, 2);
    assert!(clients.reserve_parent_switch(&mut fixture.host, 1, 23));

    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::NativeSwitchCommit { receipt: 23 },
    );
    assert_eq!(
        clients.take_switch_action(),
        Some(NativeSwitchAction::Commit {
            owner: 1,
            receipt: 23,
        })
    );
    assert!(clients.accept_parent_commit(&mut fixture.host, 1, 23));
    assert!(clients.switch_pending());
    assert_eq!(clients.active_id(), Some(1));

    request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::NativeParentSwitchCommitObserved { receipt: 23 },
    );
    assert!(clients.take_switch_action().is_none());
    assert!(clients.switch_pending());

    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::NativeParentSwitchCommitObserved { receipt: 23 },
    );
    assert_eq!(
        clients.take_switch_action(),
        Some(NativeSwitchAction::ParentCommitObserved {
            owner: 1,
            receipt: 23,
        })
    );
    assert!(clients.switch_pending());
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

#[test]
fn wait_mutation_requires_the_creating_native_connection() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _interactive = fixture.connect_interactive(&mut clients, 1);
    let _owner = fixture.connect(&mut clients, 2);
    let _other = fixture.connect(&mut clients, 3);
    let token = fixture.wait(&mut clients, 2, "owned-wait.txt");

    request(
        &mut clients,
        &mut fixture.host,
        3,
        ClientRequest::CancelWait { token },
    );
    assert!(matches!(
        fixture.host.wait_status(token.into()),
        Some(runyte::workspace::WaitStatus::Pending { .. })
    ));
    request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::CancelWait { token },
    );
    assert!(matches!(
        fixture.host.wait_status(token.into()),
        Some(runyte::workspace::WaitStatus::Cancelled { .. })
    ));
}

#[test]
fn attachment_generation_loss_cancels_only_its_parent_waits() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _interactive = fixture.connect_interactive(&mut clients, 1);
    let _parent = fixture.connect(&mut clients, 2);
    let _unrelated = fixture.connect(&mut clients, 3);
    let parent = fixture.wait(&mut clients, 2, "parent-wait.txt");
    let unrelated = fixture.wait(&mut clients, 3, "unrelated-wait.txt");
    clients
        .peers
        .get_mut(&2)
        .unwrap()
        .parent_waits
        .insert(parent);

    clients.disconnected(&mut fixture.host, 1);

    assert!(matches!(
        fixture.host.wait_status(parent.into()),
        Some(runyte::workspace::WaitStatus::Cancelled { .. })
    ));
    assert!(matches!(
        fixture.host.wait_status(unrelated.into()),
        Some(runyte::workspace::WaitStatus::Pending { .. })
    ));
    assert!(clients.peers[&2].waits.contains(&parent));
}

#[test]
fn attachment_becomes_ready_only_after_acknowledging_its_latest_issued_frame() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _interactive = fixture.connect_interactive(&mut clients, 1);
    let issued = clients.peers[&1].pending_ready_frames.unwrap().1;
    assert_eq!(clients.active_ready, None);

    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::FrameDrawn {
            frame: runyte::protocol::FrameId::from_raw(issued.get() + 1),
        },
    );
    assert_eq!(clients.active_ready, None);
    fixture.acknowledge_initial_frame(&mut clients, 1);
    assert_eq!(clients.active_ready, Some(1));
    assert_eq!(clients.peers[&1].pending_ready_frames, None);
}

#[test]
fn any_issued_pre_ready_frame_can_acknowledge_the_current_attachment() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _interactive = fixture.connect_interactive(&mut clients, 1);
    let first = clients.peers[&1].pending_ready_frames.unwrap().0;
    clients.publish_frame(&mut fixture.host);
    let (_, second) = clients.peers[&1].pending_ready_frames.unwrap();
    assert_ne!(first, second);

    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::FrameDrawn { frame: first },
    );
    assert_eq!(clients.active_ready, Some(1));
    assert_eq!(clients.peers[&1].pending_ready_frames, None);
}

#[test]
fn deferred_parent_wait_is_revalidated_after_attachment_replacement() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _old = fixture.connect_interactive(&mut clients, 1);
    fixture.acknowledge_initial_frame(&mut clients, 1);
    let launch = ParentLaunch::new(fixture.metadata.clone()).unwrap();
    fixture
        .host
        .app_mut()
        .terminals
        .set_parent_launch(launch.clone());
    let terminal = fixture
        .host
        .app_mut()
        .terminals
        .open(
            TerminalRequest {
                program: "cmd.exe".into(),
                arguments: vec!["/d".into(), "/q".into()],
                directory: fixture.root.path().to_owned(),
                label: "authority".into(),
            },
            80,
            24,
        )
        .unwrap();
    fixture.show_terminal(terminal);
    let context: ParentContext = serde_json::from_str(&launch.context(terminal)).unwrap();
    let process = fixture
        .host
        .app()
        .terminals
        .get(terminal)
        .unwrap()
        .process_id()
        .unwrap();
    let proof = Arc::new(PinnedProcess::open_peer(process).unwrap());
    assert!(
        fixture
            .host
            .app()
            .terminals
            .validates_parent(terminal, &context.capability, Some(proof.as_ref()))
            .unwrap()
    );
    let other_terminal = fixture
        .host
        .app_mut()
        .terminals
        .open(
            TerminalRequest {
                program: "cmd.exe".into(),
                arguments: vec!["/d".into(), "/q".into()],
                directory: fixture.root.path().to_owned(),
                label: "other authority".into(),
            },
            80,
            24,
        )
        .unwrap();
    let other_process = fixture
        .host
        .app()
        .terminals
        .get(other_terminal)
        .unwrap()
        .process_id()
        .unwrap();
    let other_proof = PinnedProcess::open_peer(other_process).unwrap();
    assert!(
        !fixture
            .host
            .app()
            .terminals
            .validates_parent(terminal, &context.capability, Some(&other_proof))
            .unwrap(),
        "a copied marker from a different live terminal job is not authority"
    );
    let (responses, _control) = response_channel();
    clients.connected(
        &mut fixture.host,
        ConnectedPeer {
            id: 2,
            proof,
            responses,
            interactive: false,
            geometry: runyte::app::FrameGeometry::default(),
        },
    );
    let original = fixture.root.join("original-generation.txt");
    std::fs::write(&original, "clean").unwrap();
    request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::ParentWait {
            terminal: terminal.get(),
            capability: context.capability.clone(),
            paths: vec![runyte::protocol::encode_path(&original)],
        },
    );
    let accepted = *clients.peers[&2]
        .parent_waits
        .iter()
        .next()
        .expect("the original attachment generation admits its visible terminal");
    request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::CancelWait { token: accepted },
    );
    clients.reconcile(&mut fixture.host);
    assert!(clients.peers[&2].parent_waits.is_empty());
    fixture.show_terminal(terminal);
    request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::ParentAttach {
            terminal: terminal.get(),
            capability: context.capability.clone(),
            selector: runyte::protocol::encode_path(&fixture.root.join("destination")),
            directory: runyte::protocol::encode_path(fixture.root.path()),
        },
    );
    let parent_attach = clients
        .take_parent_attach()
        .expect("the original generation admits ParentAttach authority");
    assert!(clients.parent_attach_valid(&fixture.host, &parent_attach));
    assert!(clients.reserve_parent_switch(&mut fixture.host, 1, 91));
    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::NativeSwitchCommit { receipt: 91 },
    );
    assert!(matches!(
        clients.take_switch_action(),
        Some(NativeSwitchAction::Commit {
            owner: 1,
            receipt: 91
        })
    ));
    assert!(clients.accept_parent_commit(&mut fixture.host, 1, 91));
    request(
        &mut clients,
        &mut fixture.host,
        1,
        ClientRequest::NativeParentSwitchCommitObserved { receipt: 91 },
    );
    assert!(matches!(
        clients.take_switch_action(),
        Some(NativeSwitchAction::ParentCommitObserved {
            owner: 1,
            receipt: 91
        })
    ));
    clients.disconnected(&mut fixture.host, 1);
    assert!(
        clients.parent_attach_confirmed_valid(&fixture.host, &parent_attach),
        "expected source closure after same-connection confirmation preserves retained proof"
    );
    request(&mut clients, &mut fixture.host, 2, ClientRequest::Health);
    assert!(matches!(
        clients.peers[&2].deferred,
        Some(Incoming::Request(ClientRequest::Health))
    ));
    assert!(clients.reply_parent_attach(&mut fixture.host, 2, Ok(())));
    let (released, health) = clients
        .take_released_parent_request()
        .expect("one bounded request follows ParentAttached");
    assert_eq!(released, 2);
    assert!(!clients.incoming(&mut fixture.host, released, health, no_rename));
    assert!(clients.peers[&2].deferred.is_none());
    let deferred = fixture.root.join("deferred.txt");
    std::fs::write(&deferred, "clean").unwrap();
    clients.incoming(
        &mut fixture.host,
        2,
        Incoming::Request(ClientRequest::RenameHost {
            name: "held".into(),
        }),
        |_| Ok(Box::pin(std::future::pending())),
    );
    clients.incoming(
        &mut fixture.host,
        2,
        Incoming::Request(ClientRequest::ParentWait {
            terminal: terminal.get(),
            capability: context.capability.clone(),
            paths: vec![runyte::protocol::encode_path(&deferred)],
        }),
        no_rename,
    );
    assert!(clients.peers[&2].deferred.is_some());

    clients.disconnected(&mut fixture.host, 1);
    let _replacement = fixture.connect_interactive(&mut clients, 3);
    fixture.acknowledge_initial_frame(&mut clients, 3);
    let mut metadata = fixture.metadata.clone();
    metadata.name = Some("held".into());
    clients.renamed(&mut fixture.host, 2, Ok(metadata), no_rename);

    assert!(clients.peers[&2].deferred.is_none());
    assert!(clients.peers[&2].parent_waits.is_empty());
    assert_eq!(clients.active_id(), Some(3));
    assert!(
        !clients.parent_attach_valid(&fixture.host, &parent_attach),
        "an attachment replacement cannot inherit a retained ParentAttach"
    );
    assert!(fixture.host.app_mut().terminals.close(other_terminal));
    assert!(fixture.host.app_mut().terminals.close(terminal));
    assert!(
        !fixture
            .host
            .app()
            .terminals
            .validates_parent(
                terminal,
                &context.capability,
                Some(clients.peers[&2].proof.as_ref())
            )
            .unwrap(),
        "a terminal removed before asynchronous process cleanup cannot retain authority"
    );
}

#[test]
fn completed_wait_ownership_is_retained_until_host_history_prunes_the_token() {
    let mut fixture = Fixture::new();
    let mut clients = Clients::default();
    let _owner = fixture.connect(&mut clients, 2);
    let token = fixture.wait(&mut clients, 2, "oldest.txt");
    request(
        &mut clients,
        &mut fixture.host,
        2,
        ClientRequest::CancelWait { token },
    );
    clients.reconcile(&mut fixture.host);
    assert!(clients.peers[&2].waits.contains(&token));

    for index in 0..256 {
        let path = fixture.root.join(format!("prune-{index}.txt"));
        std::fs::write(&path, "clean").unwrap();
        let (newer, _) = fixture.host.create_wait_request([path], false).unwrap();
        fixture.host.cancel_wait(newer, "fixture complete").unwrap();
    }
    assert_eq!(fixture.host.wait_status(token.into()), None);
    clients.reconcile(&mut fixture.host);
    assert!(!clients.peers[&2].waits.contains(&token));
}
