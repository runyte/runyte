// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    private_storage::Directory,
    protocol::FeatureGroup,
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{EndpointLocation, RegistrySet},
        windows_transport::{LocalServer, ResponseSender, ServerEvent},
    },
};
use std::{
    collections::HashMap,
    ffi::OsStr,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
};
use tokio::time::sleep_until;
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, WaitForSingleObject};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn location(root: &Path) -> EndpointLocation {
    std::fs::create_dir_all(root.join("project")).unwrap();
    EndpointLocation::new(
        &root.join("project"),
        root.join("endpoint"),
        RegistrySet::open(&[root.join("registry")]).unwrap(),
    )
    .unwrap()
}

fn fixture() -> (TestRuntimeRoot, EndpointLocation) {
    let root = TestRuntimeRoot::new("native-control").unwrap();
    let endpoint = location(root.path());
    (root, endpoint)
}

fn welcome() -> HostResponse {
    HostResponse::Welcome {
        protocol: crate::protocol::VERSION,
        pid: std::process::id(),
        features: vec![
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ],
        host_version: crate::protocol::CLIENT_VERSION.to_owned(),
    }
}

async fn next(server: &mut LocalServer) -> ServerEvent {
    timeout_at(Instant::now() + Duration::from_secs(10), server.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn connected(server: &mut LocalServer) -> ResponseSender {
    let ServerEvent::Connected { responses, .. } = next(server).await else {
        panic!("client was not admitted");
    };
    responses
}

#[test]
fn handshake_checks_features_protocol_peer_pid_and_host_errors() {
    runtime().block_on(async {
        let mut wrong_features = welcome();
        if let HostResponse::Welcome { features, .. } = &mut wrong_features {
            features.clear();
        }
        let mut wrong_protocol = welcome();
        if let HostResponse::Welcome { protocol, .. } = &mut wrong_protocol {
            *protocol += 1;
        }
        let mut wrong_pid = welcome();
        if let HostResponse::Welcome { pid, .. } = &mut wrong_pid {
            *pid = pid.wrapping_add(1);
        }
        for (response, expected) in [
            (welcome(), None),
            (wrong_features, Some("feature set")),
            (wrong_protocol, Some("incompatible protocol")),
            (wrong_pid, Some("authenticated native peer")),
            (
                HostResponse::Refused {
                    message: "fixture refused".into(),
                },
                Some("fixture refused"),
            ),
            (
                HostResponse::Error {
                    message: "fixture error".into(),
                },
                Some("fixture error"),
            ),
            (
                HostResponse::HostRenamed {
                    name: "unexpected".into(),
                },
                Some("unexpected workspace handshake"),
            ),
        ] {
            let (_root, endpoint) = fixture();
            let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
            let metadata = server.metadata().clone();
            let task = tokio::spawn(async move { connect_control(&metadata).await });
            let responses = connected(&mut server).await;
            responses.send(response).await.unwrap();
            let result = task.await.unwrap();
            match expected {
                None => drop(result.unwrap()),
                Some(expected) => match result {
                    Ok(_) => panic!("invalid handshake was accepted"),
                    Err(error) => assert!(error.to_string().contains(expected), "{error:#}"),
                },
            }
            drop(responses);
            server.shutdown().await.unwrap();
        }
    });
}

#[test]
fn silent_and_disconnected_handshakes_are_bounded_and_cancelled_clients_release() {
    runtime().block_on(async {
        for disconnect in [false, true] {
            let (_root, endpoint) = fixture();
            let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
            let metadata = server.metadata().clone();
            let task = tokio::spawn(async move { connect_control(&metadata).await });
            let responses = connected(&mut server).await;
            if disconnect {
                drop((responses, server));
                let Err(error) = task.await.unwrap() else {
                    panic!("disconnected handshake accepted");
                };
                assert!(
                    error.to_string().contains("disconnected during handshake"),
                    "{error:#}"
                );
            } else {
                let Err(error) = task.await.unwrap() else {
                    panic!("silent handshake accepted");
                };
                assert!(
                    error.to_string().contains("handshake timed out"),
                    "{error:#}"
                );
                drop(responses);
                server.shutdown().await.unwrap();
            }
        }
        let (_root, endpoint) = fixture();
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        let metadata = server.metadata().clone();
        let task = tokio::spawn(async move { connect_control(&metadata).await });
        let responses = connected(&mut server).await;
        task.abort();
        match task.await {
            Err(error) => assert!(error.is_cancelled()),
            Ok(_) => panic!("cancelled control client completed"),
        }
        // The transport waits for its first semantic reply before reading
        // further requests. Let that gate observe the cancelled native peer.
        responses.send(welcome()).await.unwrap();
        assert!(matches!(
            next(&mut server).await,
            ServerEvent::Disconnected { .. }
        ));
        drop(responses);
        server.shutdown().await.unwrap();
    });
}

#[test]
fn rename_requires_matching_acknowledgment_and_preserves_requested_spelling() {
    runtime().block_on(async {
        for (response, expected) in [
            (HostResponse::HostRenamed { name: "Explicit Name".into() }, None),
            (HostResponse::HostRenamed { name: "other".into() }, Some("unexpected host-rename")),
            (HostResponse::Refused { message: "name in use".into() }, Some("name in use")),
            (HostResponse::Error { message: "publication failed".into() }, Some("publication failed")),
        ] {
            let (_root, endpoint) = fixture();
            let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
            let metadata = server.metadata().clone();
            let task = tokio::spawn(async move { rename_host(&metadata, "Explicit Name").await });
            let responses = connected(&mut server).await;
            responses.send(welcome()).await.unwrap();
            assert!(matches!(next(&mut server).await, ServerEvent::Request { request: ClientRequest::RenameHost { name }, .. } if name == "Explicit Name"));
            responses.send(response).await.unwrap();
            let result = task.await.unwrap();
            match expected {
                None => result.unwrap(),
                Some(expected) => assert!(result.unwrap_err().to_string().contains(expected)),
            }
            drop(responses);
            server.shutdown().await.unwrap();
        }
        let (_root, endpoint) = fixture();
        let prepared = endpoint.prepare(None).unwrap();
        // Invalid names are refused before attempting to reach an unbound pipe.
        assert!(rename_host(prepared.metadata(), " padded").await.unwrap_err().to_string().contains("whitespace"));
    });
}

#[test]
fn rename_response_timeout_drops_outgoing_capability_without_stalling_server() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        let metadata = server.metadata().clone();
        let task = tokio::spawn(async move { rename_host(&metadata, "new").await });
        let responses = connected(&mut server).await;
        responses.send(welcome()).await.unwrap();
        assert!(matches!(
            next(&mut server).await,
            ServerEvent::Request {
                request: ClientRequest::RenameHost { .. },
                ..
            }
        ));
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("rename request timed out")
        );
        assert!(matches!(
            next(&mut server).await,
            ServerEvent::Disconnected { .. }
        ));
        drop(responses);
        server.shutdown().await.unwrap();
    });
}

#[test]
fn shutdown_preserves_refusal_and_force_stays_a_protocol_request() {
    runtime().block_on(async {
        for (force, response, expected) in [
            (
                false,
                HostResponse::Refused {
                    message: "unsaved buffers".into(),
                },
                Some("unsaved buffers"),
            ),
            (
                false,
                HostResponse::Error {
                    message: "fixture error".into(),
                },
                Some("fixture error"),
            ),
            (
                false,
                HostResponse::HostRenamed {
                    name: "unexpected".into(),
                },
                Some("unexpected shutdown"),
            ),
            (false, HostResponse::ShuttingDown, None),
            (true, HostResponse::ShuttingDown, None),
        ] {
            let (_root, endpoint) = fixture();
            let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
            let metadata = server.metadata().clone();
            let expected_metadata = metadata.clone();
            let task = tokio::spawn(async move {
                if force {
                    force_shutdown_host(&metadata).await
                } else {
                    shutdown_host(&metadata).await
                }
            });
            let responses = connected(&mut server).await;
            responses.send(welcome()).await.unwrap();
            let ServerEvent::Request { request, .. } = next(&mut server).await else {
                panic!("missing shutdown request");
            };
            assert_eq!(matches!(request, ClientRequest::ForceShutdown), force);
            assert!(matches!(
                request,
                ClientRequest::Shutdown | ClientRequest::ForceShutdown
            ));
            responses.send(response).await.unwrap();
            let result = task.await.unwrap();
            match expected {
                Some(expected) => assert!(result.unwrap_err().to_string().contains(expected)),
                None => {
                    let receipt = result.unwrap();
                    assert_eq!(receipt.metadata(), &expected_metadata);
                    assert_eq!(receipt.process(), ProcessIdentity::current().unwrap());
                    let candidate = endpoint.observe_ready().unwrap().unwrap();
                    assert!(
                        remove_stopped_observation(&receipt, &candidate)
                            .unwrap_err()
                            .to_string()
                            .contains("still running")
                    );
                    assert!(
                        await_host_stopped_until(&receipt, Instant::now())
                            .await
                            .is_err()
                    );
                    assert!(candidate.metadata().process == receipt.process());
                }
            }
            drop(responses);
            server.shutdown().await.unwrap();
        }
    });
}

#[test]
fn shutdown_timeout_has_no_receipt_and_eof_is_not_proof_of_process_exit() {
    runtime().block_on(async {
        for disconnect in [false, true] {
            let (_root, endpoint) = fixture();
            let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
            let metadata = server.metadata().clone();
            let task = tokio::spawn(async move { shutdown_host(&metadata).await });
            let responses = connected(&mut server).await;
            responses.send(welcome()).await.unwrap();
            assert!(matches!(
                next(&mut server).await,
                ServerEvent::Request {
                    request: ClientRequest::Shutdown,
                    ..
                }
            ));
            if disconnect {
                drop((responses, server));
                let receipt = task.await.unwrap().unwrap();
                assert!(receipt.peer.is_alive().unwrap());
                assert!(
                    await_host_stopped_until(&receipt, Instant::now())
                        .await
                        .is_err()
                );
            } else {
                assert!(
                    task.await
                        .unwrap()
                        .unwrap_err()
                        .to_string()
                        .contains("shutdown request timed out")
                );
                drop(responses);
                server.shutdown().await.unwrap();
            }
        }
    });
}

const HELPER_ROOT: &str = "RUNYTE_CONTROL_FIXTURE_ROOT";
const HELPER_MODE: &str = "RUNYTE_CONTROL_FIXTURE_MODE";
const HELPER_NAME: &str = "workspace::windows_lifecycle::tests::native_control_fixture";

#[test]
#[ignore = "compiled native host fixture for lifecycle process tests"]
fn native_control_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os(HELPER_ROOT).expect("fixture root"));
    let mode = std::env::var(HELPER_MODE).expect("fixture mode");
    runtime().block_on(async {
        let endpoint = location(&root);
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        if mode == "incompatible" {
            // Model an older host publication without changing production API.
            // Forced exit intentionally prevents this fixture's owner cleanup.
            let mut metadata = server.metadata().clone();
            metadata.protocol += 1;
            metadata.validate().unwrap();
            Directory::open_existing(&root.join("endpoint"), true)
                .unwrap()
                .atomic_write(
                    OsStr::new("endpoint.json"),
                    &serde_json::to_vec(&metadata).unwrap(),
                )
                .unwrap();
        }
        Directory::open_existing(&root, true)
            .unwrap()
            .atomic_write(OsStr::new("fixture-ready"), b"ready")
            .unwrap();
        let mut clients = HashMap::new();
        loop {
            let event = timeout_at(Instant::now() + Duration::from_secs(45), server.recv())
                .await
                .expect("fixture idle deadline");
            match event {
                Some(ServerEvent::Connected { id, responses, .. }) => {
                    responses.send(welcome()).await.unwrap();
                    clients.insert(id, responses);
                }
                Some(ServerEvent::Request {
                    id,
                    request: ClientRequest::Shutdown | ClientRequest::ForceShutdown,
                }) => {
                    clients
                        .get(&id)
                        .unwrap()
                        .send(HostResponse::ShuttingDown)
                        .await
                        .unwrap();
                    break;
                }
                Some(ServerEvent::Disconnected { id }) => {
                    clients.remove(&id);
                }
                Some(_) => panic!("unexpected fixture request"),
                None => break,
            }
        }
        clients.clear();
        server.shutdown().await.unwrap();
    });
}

struct FixtureChild(Child);
impl Drop for FixtureChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            unsafe {
                WaitForSingleObject(self.0.as_raw_handle(), 5000);
            }
        }
    }
}

struct NativeFixture {
    // Process cleanup precedes deletion of fixture-owned files on every path.
    child: FixtureChild,
    endpoint: EndpointLocation,
    root: TestRuntimeRoot,
}

impl NativeFixture {
    async fn start(mode: &str) -> Self {
        let (root, endpoint) = fixture();
        let child = FixtureChild(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", HELPER_NAME, "--ignored", "--nocapture"])
                .env(HELPER_ROOT, root.path())
                .env(HELPER_MODE, mode)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut fixture = Self {
            child,
            endpoint,
            root,
        };
        let directory = Directory::open_existing(fixture.root.path(), true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match directory.open_read(OsStr::new("fixture-ready")) {
                Ok(_) => return fixture,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => panic!("fixture readiness: {error}"),
            }
            assert!(
                fixture.child.0.try_wait().unwrap().is_none(),
                "fixture exited before readiness"
            );
            assert!(
                Instant::now() < deadline,
                "fixture did not publish readiness"
            );
            sleep_until(deadline.min(Instant::now() + Duration::from_millis(5))).await;
        }
    }

    fn candidate(&self) -> Candidate {
        self.endpoint.observe_ready().unwrap().unwrap()
    }
}

#[test]
fn supported_stop_receipt_observes_only_the_original_process_exit() {
    runtime().block_on(async {
        let mut fixture = NativeFixture::start("compatible").await;
        let observed = fixture.candidate();
        assert!(
            terminate_incompatible_host(&observed)
                .await
                .unwrap_err()
                .to_string()
                .contains("current protocol")
        );
        assert!(fixture.child.0.try_wait().unwrap().is_none());
        let receipt = shutdown_host(observed.metadata()).await.unwrap();
        await_host_stopped(&receipt).await.unwrap();
        assert!(!receipt.peer.is_alive().unwrap());
        assert!(fixture.child.0.try_wait().unwrap().unwrap().success());
        assert_eq!(
            remove_stopped_observation(&receipt, &observed).unwrap(),
            Removal::Missing
        );
        // A later publication at the same configured location is not the
        // process named by the stop, and cannot be retired with its receipt.
        let mut replacement = LocalServer::bind(fixture.endpoint.prepare(None).unwrap()).unwrap();
        let replacement_observation = fixture.candidate();
        await_host_stopped_until(&receipt, Instant::now())
            .await
            .unwrap();
        assert!(remove_stopped_observation(&receipt, &replacement_observation).is_err());
        assert!(fixture.endpoint.observe_ready().unwrap().is_some());
        replacement.shutdown().await.unwrap();
    });
}

#[test]
fn incompatible_force_uses_actual_peer_and_cleanup_preserves_replaced_records() {
    runtime().block_on(async {
        let mut fixture = NativeFixture::start("incompatible").await;
        let observed = fixture.candidate();
        let proof = observed
            .authenticate(Instant::now() + CONTROL_BUDGET)
            .await
            .unwrap();
        // Even an explicitly opened termination-capable handle is insufficient
        // when it belongs to a different native process object.
        let raw = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                0,
                std::process::id(),
            )
        };
        assert!(!raw.is_null());
        let unrelated = unsafe { OwnedHandle::from_raw_handle(raw) };
        assert_eq!(
            terminate_matching_handle(&proof, &unrelated)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(proof.peer().is_alive().unwrap());
        let receipt = terminate_incompatible_host(&observed).await.unwrap();
        assert_eq!(receipt.process(), proof.peer().identity());
        assert!(!fixture.child.0.try_wait().unwrap().unwrap().success());
        // Replacement with byte-identical metadata is still a different file.
        Directory::open_existing(&fixture.root.join("endpoint"), true)
            .unwrap()
            .atomic_write(
                OsStr::new("endpoint.json"),
                &serde_json::to_vec(observed.metadata()).unwrap(),
            )
            .unwrap();
        assert_eq!(
            remove_stopped_observation(&receipt, &observed).unwrap(),
            Removal::Changed
        );
        let fresh = fixture.candidate();
        assert_eq!(
            remove_stopped_observation(&receipt, &fresh).unwrap(),
            Removal::Removed
        );
        assert_eq!(
            remove_stopped_observation(&receipt, &fresh).unwrap(),
            Removal::Missing
        );
        // Registry scope is independent; removing ready did not infer or clear
        // any other record merely because its metadata named this project.
        assert_eq!(
            RegistrySet::open(&[fixture.root.join("registry")])
                .unwrap()
                .scan_namespaces()
                .candidates
                .len(),
            1
        );
    });
}

#[test]
fn missing_native_peer_never_authorizes_incompatible_force_or_cleanup() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        let mut metadata = server.metadata().clone();
        let original = endpoint.observe_ready().unwrap().unwrap();
        metadata.protocol += 1;
        // A syntactically valid, private observation with a mismatching native
        // identity must not gain authority over the reachable current process.
        metadata.process.creation_time ^= 1;
        let parent = endpoint.ready_record().parent().unwrap().to_owned();
        Directory::open_existing(&parent, true)
            .unwrap()
            .atomic_write(
                OsStr::new("endpoint.json"),
                &serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        let candidate = endpoint.observe_ready().unwrap().unwrap();
        assert!(terminate_incompatible_host(&candidate).await.is_err());
        assert!(original.inspect_process().is_ok());
        assert_eq!(
            endpoint.observe_ready().unwrap().unwrap().metadata(),
            &metadata
        );
        // An actual peer proof still cannot authorize killing this lifecycle
        // client itself, even with an incompatible version in its publication.
        metadata.process = ProcessIdentity::current().unwrap();
        Directory::open_existing(&parent, true)
            .unwrap()
            .atomic_write(
                OsStr::new("endpoint.json"),
                &serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        let candidate = endpoint.observe_ready().unwrap().unwrap();
        assert!(
            terminate_incompatible_host(&candidate)
                .await
                .unwrap_err()
                .to_string()
                .contains("current lifecycle client")
        );
        server.shutdown().await.unwrap();
    });
}
