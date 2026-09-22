// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    app::FrameGeometry,
    input::{InputEvent, KeyStroke},
    protocol::{
        ClientRequest, FeatureGroup, HostResponse, SnapshotRow, TransportChange, WaitStatus,
        encode_path,
    },
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::EndpointMetadata,
        windows_lifecycle::{await_host_stopped, connect_control, force_shutdown_host},
        windows_location::{CapturedRoots, EXPECTED_LAYOUT_ENV, LocationInputs, ResolvedLayout},
        windows_process_identity::PinnedProcess,
        windows_transport::{BufferedLocalClient, LocalClient},
    },
};
use std::{
    fs,
    io::{Read, Write},
    mem::size_of,
    os::windows::{
        io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant as StdInstant},
};
use tokio::time::{Instant, sleep, timeout};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{CREATE_NO_WINDOW, GetExitCodeProcess, WaitForSingleObject},
    },
};

const BUDGET: Duration = Duration::from_secs(10);

// These hosts have no process plugins, an empty executable search path, disabled
// LSP and no terminal requests. The guard owns their exact std Child handle.
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let waited = unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5000) };
        if !std::thread::panicking() {
            assert_eq!(
                waited, WAIT_OBJECT_0,
                "fixture host did not exit before storage cleanup"
            );
        }
    }
}

struct Fixture {
    root: TestRuntimeRoot,
    project: PathBuf,
    config: PathBuf,
    layout: ResolvedLayout,
}
impl Fixture {
    fn new() -> Self {
        let root = TestRuntimeRoot::new("native-real-host").unwrap();
        let project = root.create_private_dir("project").unwrap();
        let config_root = root.create_private_dir("config").unwrap();
        let runtime = root.create_private_dir("runtime").unwrap();
        let cache = root.create_private_dir("cache").unwrap();
        let config = config_root.join("config.yaml");
        fs::write(
            &config,
            "lsp:\n  enable: false\nworkspace:\n  idle_retirement_minutes: 0\n",
        )
        .unwrap();
        let layout = ResolvedLayout::resolve(LocationInputs {
            project_root: project.clone(),
            state_root: project.join(".runyte"),
            reserved_user_roots: vec![config_root],
            roots: CapturedRoots {
                runtime_root: Some(runtime),
                cache_home: Some(cache),
                local_app_data: None,
                inventory_override: Some(root.join("inventory")),
            },
        })
        .unwrap();
        Self {
            root,
            project,
            config,
            layout,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
        command
            .args(["--serve", "--detached-host", "--project-root"])
            .arg(&self.project)
            .arg("--config")
            .arg(&self.config)
            .current_dir(&self.project)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("PATH", "")
            .env_remove("RUNYTE_PARENT_CONTEXT")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(self.root.join("host-stderr")).unwrap());
        for (name, value) in self.layout.detached_environment().unwrap() {
            if let Some(value) = value {
                command.env(name, value);
            } else {
                command.env_remove(name);
            }
        }
        command
    }

    fn foreground_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
        command
            .arg("--serve")
            .arg("--config")
            .arg(&self.config)
            .current_dir(&self.project)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_RUNTIME_DIR", self.root.join("runtime"))
            .env("RUNYTE_ALL_HOSTS_DIR", self.root.join("inventory"))
            .env_remove(EXPECTED_LAYOUT_ENV)
            .env("PATH", "")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(self.root.join("foreground-stderr")).unwrap());
        command
    }

    async fn start(&self) -> (Process, EndpointMetadata) {
        let mut process = Process(self.command().spawn().unwrap());
        let deadline = Instant::now() + BUDGET;
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                let mut detail = Vec::new();
                fs::File::open(self.root.join("host-stderr"))
                    .unwrap()
                    .take(4096)
                    .read_to_end(&mut detail)
                    .unwrap();
                panic!(
                    "native host exited before readiness ({status}): {}",
                    String::from_utf8_lossy(&detail)
                );
            }
            if let Ok(view) = self.layout.discovery_view(false)
                && let Some(candidate) = view
                    .observe_ready(
                        self.layout.project_root(),
                        self.layout.endpoint_directory().to_owned(),
                    )
                    .unwrap()
                && connect_control(candidate.metadata()).await.is_ok()
            {
                return (process, candidate.metadata().clone());
            }
            assert!(
                Instant::now() < deadline,
                "real native host readiness timed out"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }

    async fn exited(&self, process: &mut Process) {
        let deadline = Instant::now() + BUDGET;
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(status.success(), "host status: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "native host did not exit");
            sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !self
                .layout
                .endpoint_directory()
                .join("endpoint.json")
                .exists()
        );
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn response(client: &mut LocalClient) -> HostResponse {
    timeout(BUDGET, client.recv())
        .await
        .unwrap()
        .unwrap()
        .expect("host closed before response")
}

async fn request(client: &mut LocalClient, request: ClientRequest) -> HostResponse {
    timeout(BUDGET, client.send(&request))
        .await
        .unwrap()
        .unwrap();
    response(client).await
}

async fn buffered_response(client: &mut BufferedLocalClient, stage: &str) -> HostResponse {
    timeout(BUDGET, client.recv())
        .await
        .unwrap_or_else(|_| panic!("native interactive response timed out at {stage}"))
        .unwrap()
        .expect("native interactive peer closed before response")
}

async fn buffered_semantic_response(client: &mut BufferedLocalClient, stage: &str) -> HostResponse {
    timeout(BUDGET, async {
        loop {
            let response = client
                .recv()
                .await
                .unwrap()
                .expect("native interactive peer closed");
            if !matches!(
                response,
                HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
            ) {
                return response;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("native interactive semantic response timed out at {stage}"))
}

async fn buffered_frame(
    client: &mut BufferedLocalClient,
    stage: &str,
) -> runyte::protocol::HostFrame {
    timeout(BUDGET, async {
        loop {
            let response = client
                .recv()
                .await
                .unwrap()
                .expect("native interactive peer closed");
            if let HostResponse::Frame { frame } = response {
                return *frame;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("native interactive frame timed out at {stage}"))
}

async fn buffered_invoke(
    client: &mut BufferedLocalClient,
    frame: &runyte::protocol::HostFrame,
    name: &str,
) {
    client
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest::at(
                name,
                frame.id,
                frame.active_buffer,
                frame.active_revision,
            ),
        })
        .await
        .unwrap();
}

async fn buffered_invoke_current(
    client: &mut BufferedLocalClient,
    mut frame: runyte::protocol::HostFrame,
    name: &str,
) -> HostResponse {
    timeout(BUDGET, async {
        loop {
            buffered_invoke(client, &frame, name).await;
            let response = buffered_semantic_response(client, name).await;
            match response {
                HostResponse::Error { ref message }
                    if message.starts_with("stale editor frame:") =>
                {
                    client.send(&ClientRequest::Resynchronize).await.unwrap();
                    frame = buffered_frame(client, "retry current command frame").await;
                }
                response => return response,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("native {name} command could not obtain a current frame"))
}

fn frame_contains(frame: &runyte::protocol::HostFrame, expected: &str) -> bool {
    frame_text(frame).contains(expected)
}

fn frame_text(frame: &runyte::protocol::HostFrame) -> String {
    frame
        .editor
        .panes
        .iter()
        .flat_map(|pane| &pane.rows)
        .filter_map(|row| match row {
            SnapshotRow::Text(row) => Some(
                row.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn native_host_edits_unsaved_revisions_saves_and_refuses_dirty_shutdown() {
    runtime().block_on(async {
        let fixture = Fixture::new();
        let path = fixture.project.join("literal 雪 $; '.txt");
        fs::write(&path, "before").unwrap();
        let (mut process, metadata) = fixture.start().await;
        let mut client = connect_control(&metadata).await.unwrap();
        let HostResponse::Opened { buffers } = request(
            &mut client,
            ClientRequest::OpenBuffers {
                paths: vec![encode_path(&path)],
                activate: true,
            },
        )
        .await
        else {
            panic!("expected opened buffer")
        };
        let buffer = buffers[0];
        let HostResponse::Buffer { buffer: contents } =
            request(&mut client, ClientRequest::ReadBuffer { buffer }).await
        else {
            panic!("expected buffer")
        };
        assert_eq!(contents.text, "before");
        assert!(matches!(
            request(
                &mut client,
                ClientRequest::Invoke {
                    command: runyte::protocol::CommandRequest::at(
                        "select-all",
                        serde_json::from_str("1").unwrap(),
                        buffer,
                        contents.metadata.revision,
                    ),
                }
            )
            .await,
            HostResponse::Error { .. }
        ));
        let mutation = ClientRequest::ApplyTransaction {
            buffer,
            expected: contents.metadata.revision,
            changes: vec![TransportChange {
                from: 0,
                to: 6,
                text: "after 雪".into(),
            }],
        };
        assert!(matches!(
            request(&mut client, mutation.clone()).await,
            HostResponse::TransactionApplied { .. }
        ));
        assert!(matches!(
            request(&mut client, mutation).await,
            HostResponse::StaleRevision { .. }
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "before");
        assert!(matches!(
            request(&mut client, ClientRequest::Shutdown).await,
            HostResponse::Refused { .. }
        ));
        assert!(matches!(
            request(&mut client, ClientRequest::SaveBuffer { buffer }).await,
            HostResponse::Saved { .. }
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "after 雪");
        assert_eq!(
            request(&mut client, ClientRequest::Shutdown).await,
            HostResponse::ShuttingDown
        );
        drop(client);
        fixture.exited(&mut process).await;
    });
}

#[test]
fn native_host_disconnect_cancels_only_that_clients_waits() {
    runtime().block_on(async {
        let fixture = Fixture::new();
        let path = fixture.project.join("wait.txt");
        fs::write(&path, "clean").unwrap();
        let (mut process, metadata) = fixture.start().await;
        let mut first = connect_control(&metadata).await.unwrap();
        let mut second = connect_control(&metadata).await.unwrap();
        let create = ClientRequest::CreateWait {
            paths: vec![encode_path(&path)],
        };
        let HostResponse::WaitCreated { token: one, .. } =
            request(&mut first, create.clone()).await
        else {
            panic!("wait one")
        };
        let HostResponse::WaitCreated {
            token: two,
            buffers,
            ..
        } = request(&mut second, create).await
        else {
            panic!("wait two")
        };
        assert!(matches!(
            request(&mut second, ClientRequest::Shutdown).await,
            HostResponse::Refused { .. }
        ));
        drop(first);
        let deadline = Instant::now() + BUDGET;
        loop {
            if matches!(
                request(&mut second, ClientRequest::WaitStatus { token: one }).await,
                HostResponse::WaitState {
                    status: WaitStatus::Cancelled { .. },
                    ..
                }
            ) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "disconnected wait was not cancelled"
            );
            sleep(Duration::from_millis(10)).await;
        }
        assert!(matches!(
            request(&mut second, ClientRequest::WaitStatus { token: two }).await,
            HostResponse::WaitState {
                status: WaitStatus::Pending { .. },
                ..
            }
        ));
        assert!(matches!(
            request(
                &mut second,
                ClientRequest::CompleteWaitBuffer {
                    token: two,
                    buffer: buffers[0]
                }
            )
            .await,
            HostResponse::WaitState {
                status: WaitStatus::Completed,
                ..
            }
        ));
        drop(second);
        let mut observer = connect_control(&metadata).await.unwrap();
        assert!(matches!(
            request(&mut observer, ClientRequest::WaitStatus { token: two }).await,
            HostResponse::WaitState {
                status: WaitStatus::Completed,
                ..
            }
        ));
        let stopped = force_shutdown_host(&metadata).await.unwrap();
        await_host_stopped(&stopped).await.unwrap();
        fixture.exited(&mut process).await;
    });
}

#[test]
fn native_host_control_handshake_rename_order_and_interactive_refusal() {
    runtime().block_on(async {
        let fixture = Fixture::new();
        let (mut process, metadata) = fixture.start().await;
        let mut client = LocalClient::connect(&metadata, FrameGeometry::default(), false)
            .await
            .unwrap();
        let HostResponse::Welcome {
            protocol,
            pid,
            features,
            ..
        } = response(&mut client).await
        else {
            panic!("expected welcome")
        };
        assert_eq!(pid, process.0.id());
        assert_eq!(protocol, runyte::protocol::VERSION);
        assert_eq!(
            features,
            vec![
                FeatureGroup::Control,
                FeatureGroup::Buffers,
                FeatureGroup::Wait
            ]
        );
        let mut interactive = LocalClient::connect(&metadata, FrameGeometry::default(), true)
            .await
            .unwrap();
        assert!(matches!(
            response(&mut interactive).await,
            HostResponse::Welcome { features, .. } if features == vec![
                FeatureGroup::Snapshots,
                FeatureGroup::Input,
                FeatureGroup::Buffers,
                FeatureGroup::Wait,
            ]
        ));
        assert!(matches!(
            response(&mut interactive).await,
            HostResponse::Frame { .. }
        ));
        let mut second_interactive =
            LocalClient::connect(&metadata, FrameGeometry::default(), true)
                .await
                .unwrap();
        assert!(matches!(
            response(&mut second_interactive).await,
            HostResponse::Refused { .. }
        ));
        client
            .send(&ClientRequest::RenameHost {
                name: "native renamed".into(),
            })
            .await
            .unwrap();
        client.send(&ClientRequest::Health).await.unwrap();
        assert_eq!(
            response(&mut client).await,
            HostResponse::HostRenamed {
                name: "native renamed".into()
            }
        );
        assert!(matches!(
            response(&mut client).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        ));
        let current = fixture
            .layout
            .discovery_view(false)
            .unwrap()
            .observe_ready(
                fixture.layout.project_root(),
                fixture.layout.endpoint_directory().to_owned(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(current.metadata().name.as_deref(), Some("native renamed"));
        assert!(matches!(
            request(
                &mut client,
                ClientRequest::RenameHost {
                    name: "bad\nname".into()
                }
            )
            .await,
            HostResponse::Error { .. }
        ));
        assert!(matches!(
            request(&mut client, ClientRequest::Health).await,
            HostResponse::Health { .. }
        ));
        drop(interactive);
        let stopped = force_shutdown_host(&metadata).await.unwrap();
        await_host_stopped(&stopped).await.unwrap();
        fixture.exited(&mut process).await;
    });
}

#[test]
fn native_host_buffered_interactive_wire_is_single_owned_and_waits_stay_peer_scoped() {
    runtime().block_on(async {
        let fixture = Fixture::new();
        let wait_path = fixture.project.join("attached-wait.txt");
        fs::write(&wait_path, "clean").unwrap();
        let edit_path = fixture.project.join("attached-edit.txt");
        fs::write(&edit_path, "before").unwrap();
        let (mut process, metadata) = fixture.start().await;
        let mut control = connect_control(&metadata).await.unwrap();
        assert!(matches!(request(&mut control, ClientRequest::OpenBuffers {
            paths: vec![encode_path(&edit_path)], activate: true,
        }).await, HostResponse::Opened { .. }));
        let geometry = FrameGeometry {
            screen: runyte::layout::Rect { x: 0, y: 0, width: 80, height: 24 },
            editor: runyte::layout::Rect { x: 0, y: 0, width: 80, height: 22 },
            status: runyte::layout::Rect { x: 0, y: 22, width: 80, height: 1 },
            message: runyte::layout::Rect { x: 0, y: 23, width: 80, height: 1 },
        };
        let mut attached = BufferedLocalClient::connect_with_handoff(&metadata, geometry, false)
            .await.unwrap();
        assert!(matches!(
            timeout(BUDGET, attached.recv_handshake()).await.unwrap().unwrap(),
            Some(HostResponse::Welcome { features, .. }) if features == vec![
                FeatureGroup::Snapshots, FeatureGroup::Input, FeatureGroup::Buffers, FeatureGroup::Wait,
            ]
        ));
        let HostResponse::Frame { frame: initial } = buffered_response(&mut attached, "initial frame").await else {
            panic!("native interactive handshake must be followed by a complete frame")
        };
        let mut second = LocalClient::connect(&metadata, geometry, true).await.unwrap();
        assert!(matches!(response(&mut second).await, HostResponse::Refused { .. }));
        assert!(matches!(
            request(&mut control, ClientRequest::Health).await,
            HostResponse::Health { interactive_attached: true, .. }
        ));
        assert!(matches!(
            request(&mut control, ClientRequest::Input {
                event: InputEvent::Text("untrusted".into()).into(),
                repeated: false,
                presented_frame: Some(initial.id),
            }).await,
            HostResponse::Error { .. }
        ));
        attached.send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::char('i')).into(),
            repeated: false,
            presented_frame: Some(initial.id),
        }).await.unwrap();
        assert!(matches!(buffered_response(&mut attached, "enter insert frame").await, HostResponse::Frame { .. }));
        attached.send(&ClientRequest::Input {
            event: InputEvent::Text("native interactive text".into()).into(),
            repeated: false,
            presented_frame: None,
        }).await.unwrap();
        let mut last_frame = None;
        timeout(BUDGET, async {
            loop {
                let response = attached.recv().await.unwrap().expect("native interactive peer closed");
                if let HostResponse::Frame { frame } = response {
                    if frame_contains(&frame, "native interactive text") { break; }
                    last_frame = Some(format!("mode={:?}, revision={:?}, text={:?}",
                        frame.editor.mode, frame.active_revision, frame_text(&frame)));
                }
            }
        }).await.unwrap_or_else(|_| panic!("native input was not rendered; last frame: {last_frame:?}"));
        let resized = FrameGeometry {
            screen: runyte::layout::Rect { width: 100, ..geometry.screen },
            editor: runyte::layout::Rect { width: 100, ..geometry.editor },
            status: runyte::layout::Rect { width: 100, ..geometry.status },
            message: runyte::layout::Rect { width: 100, ..geometry.message },
        };
        attached.send(&ClientRequest::Resize { geometry: resized.into() }).await.unwrap();
        timeout(BUDGET, async {
            loop {
                let response = attached.recv().await.unwrap().expect("native interactive peer closed");
                if let HostResponse::Frame { frame } = response
                    && frame.editor.geometry.screen.width == 100 { break; }
            }
        }).await.expect("resize did not yield the requested native geometry");
        attached.send(&ClientRequest::CreateWait { paths: vec![encode_path(&wait_path)] }).await.unwrap();
        let HostResponse::WaitCreated { token: attached_wait, .. } = buffered_semantic_response(&mut attached, "CreateWait").await else {
            panic!("interactive wait was not admitted")
        };
        let HostResponse::WaitCreated { token: control_wait, .. } = request(&mut control,
            ClientRequest::CreateWait { paths: vec![encode_path(&wait_path)] }).await else {
            panic!("independent control wait was not admitted")
        };
        drop(attached);
        let deadline = Instant::now() + BUDGET;
        loop {
            if matches!(request(&mut control, ClientRequest::WaitStatus { token: attached_wait }).await,
                HostResponse::WaitState { status: WaitStatus::Cancelled { .. }, .. }) { break; }
            assert!(Instant::now() < deadline, "disconnected interactive wait was not cancelled");
            sleep(Duration::from_millis(10)).await;
        }
        assert!(matches!(request(&mut control, ClientRequest::WaitStatus { token: control_wait }).await,
            HostResponse::WaitState { status: WaitStatus::Pending { .. }, .. }));
        assert!(matches!(request(&mut control, ClientRequest::Health).await,
            HostResponse::Health { interactive_attached: false, .. }));
        let mut reattached = BufferedLocalClient::connect_with_handoff(&metadata, geometry, false).await.unwrap();
        assert!(matches!(timeout(BUDGET, reattached.recv_handshake()).await.unwrap().unwrap(),
            Some(HostResponse::Welcome { .. })));
        assert!(matches!(buffered_response(&mut reattached, "reattached initial frame").await, HostResponse::Frame { .. }));
        reattached.send(&ClientRequest::Detach).await.unwrap();
        assert!(matches!(buffered_semantic_response(&mut reattached, "Detach").await, HostResponse::Detached { .. }));
        drop(reattached);
        let stopped = force_shutdown_host(&metadata).await.unwrap();
        await_host_stopped(&stopped).await.unwrap();
        fixture.exited(&mut process).await;
    });
}

#[test]
fn native_host_editor_quit_finishes_attached_wait_before_final_reply_and_preserves_other_wait() {
    runtime().block_on(async {
        let fixture = Fixture::new();
        let path = fixture.project.join("quit-wait.txt");
        fs::write(&path, "clean").unwrap();
        let (mut process, metadata) = fixture.start().await;
        let mut control = connect_control(&metadata).await.unwrap();
        let HostResponse::WaitCreated { token: control_wait, buffers, .. } = request(&mut control,
            ClientRequest::CreateWait { paths: vec![encode_path(&path)] }).await else {
            panic!("control wait was not created before interactive attachment")
        };
        let mut attached = BufferedLocalClient::connect_with_handoff(
            &metadata, FrameGeometry::default(), true,
        ).await.unwrap();
        assert!(matches!(timeout(BUDGET, attached.recv_handshake()).await.unwrap().unwrap(),
            Some(HostResponse::Welcome { .. })));
        let initial = buffered_frame(&mut attached, "quit initial").await;
        assert!(matches!(buffered_invoke_current(&mut attached, initial, "quit-here").await,
            HostResponse::CommandResult { outcome: runyte::protocol::CommandOutcome::UserError(message) }
                if message.contains("runyte()")));
        assert!(matches!(request(&mut control, ClientRequest::Health).await,
            HostResponse::Health { interactive_attached: true, .. }));
        attached.send(&ClientRequest::CreateWait { paths: vec![encode_path(&path)] }).await.unwrap();
        let HostResponse::WaitCreated { token: attached_wait, .. } =
            buffered_semantic_response(&mut attached, "attached quit wait").await else {
                panic!("attached wait was not created")
            };
        attached.send(&ClientRequest::Resynchronize).await.unwrap();
        let frame = buffered_frame(&mut attached, "pre-quit frame").await;
        let first_quit = buffered_invoke_current(&mut attached, frame, "quit").await;
        assert!(matches!(first_quit, HostResponse::CommandResult { .. }),
            "first quit response: {first_quit:?}");
        assert!(matches!(buffered_semantic_response(&mut attached, "attached wait completion").await,
            HostResponse::WaitState { token, status: WaitStatus::Completed, interactive_attached: false }
                if token == attached_wait));
        assert!(matches!(request(&mut control, ClientRequest::WaitStatus { token: control_wait }).await,
            HostResponse::WaitState { status: WaitStatus::Pending { .. }, .. }));
        assert!(matches!(request(&mut control, ClientRequest::Health).await,
            HostResponse::Health { interactive_attached: true, .. }));
        assert!(matches!(request(&mut control, ClientRequest::CompleteWaitBuffer {
            token: control_wait, buffer: buffers[0],
        }).await, HostResponse::WaitState { status: WaitStatus::Completed, .. }));
        attached.send(&ClientRequest::CreateWait { paths: vec![encode_path(&path)] }).await.unwrap();
        let HostResponse::WaitCreated { token: final_wait, .. } =
            buffered_semantic_response(&mut attached, "final attached quit wait").await else {
                panic!("final attached wait was not created")
            };
        attached.send(&ClientRequest::Resynchronize).await.unwrap();
        let frame = buffered_frame(&mut attached, "final quit frame").await;
        assert!(matches!(buffered_invoke_current(&mut attached, frame, "quit").await,
            HostResponse::CommandResult { .. }));
        assert!(matches!(buffered_semantic_response(&mut attached, "final wait completion").await,
            HostResponse::WaitState { token, status: WaitStatus::Completed, interactive_attached: false }
                if token == final_wait));
        assert!(matches!(buffered_semantic_response(&mut attached, "final quit reply").await,
            HostResponse::ShuttingDown));
        drop(attached);
        drop(control);
        fixture.exited(&mut process).await;
    });
}

#[test]
fn native_host_layout_mismatch_refuses_before_publication() {
    runtime().block_on(async {
        let fixture = Fixture::new();
        let mut command = fixture.command();
        command.env(EXPECTED_LAYOUT_ENV, "mismatched-layout");
        let mut process = Process(command.spawn().unwrap());
        let deadline = Instant::now() + BUDGET;
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "invalid host launch did not refuse"
            );
            sleep(Duration::from_millis(10)).await;
        }
        assert!(!fixture.layout.endpoint_directory().exists());
        assert!(!fixture.root.join("inventory").exists());
    });
}

#[test]
fn foreground_serve_requires_a_discoverable_or_explicit_project() {
    runtime().block_on(async {
        let fixture = Fixture::new();
        let mut process = Process(fixture.foreground_command().spawn().unwrap());
        let deadline = Instant::now() + BUDGET;
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "projectless --serve did not refuse"
            );
            sleep(Duration::from_millis(10)).await;
        }
        assert!(
            fs::read_to_string(fixture.root.join("foreground-stderr"))
                .unwrap()
                .contains("--project-root")
        );
        assert!(!fixture.layout.endpoint_directory().exists());
    });
}

#[test]
fn foreground_serve_discovers_an_existing_project_without_prompting() {
    runtime().block_on(async {
        let mut fixture = Fixture::new();
        let marker = TestRuntimeRoot::new_in("projectstate", &fixture.project).unwrap();
        let state_name = marker.path().file_name().unwrap().to_str().unwrap();
        fs::write(
            &fixture.config,
            format!("lsp:\n  enable: false\nworkspace:\n  state: {state_name}\n  idle_retirement_minutes: 0\n"),
        )
        .unwrap();
        fixture.layout = ResolvedLayout::resolve(LocationInputs {
            project_root: fixture.project.clone(),
            state_root: marker.path().to_path_buf(),
            reserved_user_roots: vec![fixture.root.join("config")],
            roots: CapturedRoots {
                runtime_root: Some(fixture.root.join("runtime")),
                cache_home: Some(fixture.root.join("cache")),
                local_app_data: None,
                inventory_override: Some(fixture.root.join("inventory")),
            },
        })
        .unwrap();
        let mut process = Process(fixture.foreground_command().spawn().unwrap());
        let deadline = Instant::now() + BUDGET;
        let metadata = loop {
            if let Ok(view) = fixture.layout.discovery_view(false)
                && let Some(candidate) = view
                    .observe_ready(
                        &fixture.project,
                        fixture.layout.endpoint_directory().to_owned(),
                    )
                    .unwrap()
                && connect_control(candidate.metadata()).await.is_ok()
            {
                break candidate.metadata().clone();
            }
            assert!(
                Instant::now() < deadline,
                "foreground project was not discovered"
            );
            sleep(Duration::from_millis(10)).await;
        };
        assert!(process.0.try_wait().unwrap().is_none());
        let stopped = force_shutdown_host(&metadata).await.unwrap();
        await_host_stopped(&stopped).await.unwrap();
        fixture.exited(&mut process).await;
    });
}

const FOREGROUND_HELPER: &str = "foreground_parent_helper";
const FOREGROUND_MODE: &str = "RUNYTE_TEST_FOREGROUND_HELPER_MODE";
const FOREGROUND_ROOT: &str = "RUNYTE_TEST_FOREGROUND_HELPER_ROOT";

/// Owns the parent helper and every process it creates through exit/unwind.
/// Storage is retained if Windows cannot prove the job has drained.
struct ForegroundTree {
    fixture: Option<Fixture>,
    job: OwnedHandle,
    joined: bool,
}

impl ForegroundTree {
    fn new() -> Self {
        let fixture = Fixture::new();
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        assert!(!raw.is_null(), "cannot create foreground fixture job");
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        assert_ne!(
            unsafe {
                SetInformationJobObject(
                    job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            },
            0,
            "cannot arm foreground fixture job"
        );
        Self {
            fixture: Some(fixture),
            job,
            joined: false,
        }
    }

    fn fixture(&self) -> &Fixture {
        self.fixture.as_ref().unwrap()
    }

    fn shutdown(&mut self) -> std::io::Result<()> {
        if self.joined {
            return Ok(());
        }
        if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let deadline = StdInstant::now() + BUDGET;
        loop {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            if unsafe {
                QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if accounting.ActiveProcesses == 0 {
                self.joined = true;
                return Ok(());
            }
            if StdInstant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "foreground fixture job still has live processes",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for ForegroundTree {
    fn drop(&mut self) {
        if self.shutdown().is_err()
            && let Some(fixture) = self.fixture.take()
        {
            std::mem::forget(fixture);
        }
    }
}

fn publish_marker(root: &Path, name: &str, bytes: &[u8]) {
    let pending = root.join(format!("{name}.pending"));
    fs::write(&pending, bytes).unwrap();
    fs::rename(pending, root.join(name)).unwrap();
}

async fn wait_marker(root: &Path, name: &str) {
    let deadline = Instant::now() + BUDGET;
    while !root.join(name).exists() {
        assert!(Instant::now() < deadline, "fixture did not publish {name}");
        sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_child_exit(process: &mut Process) {
    let deadline = Instant::now() + BUDGET;
    loop {
        if let Some(status) = process.0.try_wait().unwrap() {
            assert!(
                status.success(),
                "foreground parent fixture failed: {status}"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "foreground parent fixture did not exit"
        );
        sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_pinned_exit(process: &PinnedProcess) {
    let deadline = Instant::now() + BUDGET;
    while process.is_alive().unwrap() {
        assert!(Instant::now() < deadline, "supervised host did not exit");
        sleep(Duration::from_millis(10)).await;
    }
}

fn pinned_exit_code(process: &PinnedProcess) -> u32 {
    let mut code = 0;
    assert_ne!(
        unsafe { GetExitCodeProcess(process.as_handle().as_raw_handle(), &mut code) },
        0
    );
    code
}

fn spawn_foreground_parent(tree: &ForegroundTree, mode: &str) -> Process {
    let fixture = tree.fixture();
    Process(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FOREGROUND_HELPER, "--ignored", "--nocapture"])
            .env(FOREGROUND_MODE, mode)
            .env(FOREGROUND_ROOT, fixture.root.path())
            .env("XDG_CONFIG_HOME", fixture.root.join("config"))
            .creation_flags(CREATE_NO_WINDOW)
            .current_dir(&fixture.project)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(fs::File::create(fixture.root.join("parent-stderr")).unwrap())
            .spawn()
            .unwrap(),
    )
}

#[test]
#[ignore = "reexecuted as the natural parent of a real foreground host"]
fn foreground_parent_helper() {
    let Some(mode) = std::env::var_os(FOREGROUND_MODE) else {
        return;
    };
    let mode = mode.to_str().unwrap();
    assert!(matches!(mode, "startup" | "serving" | "detached"));
    let root = PathBuf::from(std::env::var_os(FOREGROUND_ROOT).unwrap());
    publish_marker(&root, "helper-ready", b"ready");
    let mut go = [0u8; 1];
    std::io::stdin().read_exact(&mut go).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .args(["--serve", "--project-root"])
        .arg(root.join("project"))
        .arg("--config")
        .arg(root.join("config/config.yaml"))
        .current_dir(root.join("project"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove(EXPECTED_LAYOUT_ENV)
        .env_remove("RUNYTE_TEST_FOREGROUND_PARENT_BARRIER")
        .env("PATH", "")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(root.join("foreground-stderr")).unwrap());
    if mode == "startup" {
        command.env("RUNYTE_TEST_FOREGROUND_PARENT_BARRIER", &root);
    }
    if mode == "detached" {
        command.arg("--detached-host");
        let layout = ResolvedLayout::resolve(LocationInputs {
            project_root: root.join("project"),
            state_root: root.join("project/.runyte"),
            reserved_user_roots: vec![root.join("config")],
            roots: CapturedRoots {
                runtime_root: Some(root.join("runtime")),
                cache_home: Some(root.join("cache")),
                local_app_data: None,
                inventory_override: Some(root.join("inventory")),
            },
        })
        .unwrap();
        for (name, value) in layout.detached_environment().unwrap() {
            if let Some(value) = value {
                command.env(name, value);
            } else {
                command.env_remove(name);
            }
        }
    }
    let child = command.spawn().unwrap();
    publish_marker(&root, "foreground-pid", child.id().to_string().as_bytes());
    drop(child); // The outer job owns the live process and its descendants.
    // The outer owner may first wait for this host and then start an
    // unrelated host. Those are separate bounded phases before it releases
    // the parent, so this helper's deadline covers their combined budget.
    let deadline = StdInstant::now() + BUDGET * 3;
    while !root.join("exit-parent").exists() {
        assert!(StdInstant::now() < deadline, "parent release timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

async fn admitted_parent(tree: &ForegroundTree, mode: &str) -> Process {
    let mut parent = spawn_foreground_parent(tree, mode);
    wait_marker(tree.fixture().root.path(), "helper-ready").await;
    assert_ne!(
        unsafe { AssignProcessToJobObject(tree.job.as_raw_handle(), parent.0.as_raw_handle()) },
        0,
        "cannot own foreground parent and its child"
    );
    parent.0.stdin.as_mut().unwrap().write_all(b"g").unwrap();
    wait_marker(tree.fixture().root.path(), "foreground-pid").await;
    parent
}

fn foreground_process(tree: &ForegroundTree) -> PinnedProcess {
    let pid = fs::read_to_string(tree.fixture().root.join("foreground-pid"))
        .unwrap()
        .parse()
        .unwrap();
    PinnedProcess::open_peer(pid).unwrap()
}

#[cfg(debug_assertions)]
#[test]
fn foreground_parent_loss_before_publication_refuses_startup() {
    runtime().block_on(async {
        let mut tree = ForegroundTree::new();
        let mut parent = admitted_parent(&tree, "startup").await;
        wait_marker(tree.fixture().root.path(), "parent-pinned").await;
        let host = foreground_process(&tree);
        assert!(
            !tree
                .fixture()
                .layout
                .endpoint_directory()
                .join("endpoint.json")
                .exists()
        );
        publish_marker(tree.fixture().root.path(), "exit-parent", b"go");
        wait_child_exit(&mut parent).await;
        publish_marker(tree.fixture().root.path(), "release-parent-startup", b"go");
        wait_pinned_exit(&host).await;
        assert_ne!(pinned_exit_code(&host), 0);
        assert!(
            fs::read_to_string(tree.fixture().root.join("foreground-stderr"))
                .unwrap()
                .contains("foreground parent exited before host publication")
        );
        assert!(
            !tree
                .fixture()
                .layout
                .endpoint_directory()
                .join("endpoint.json")
                .exists()
        );
        tree.shutdown().unwrap();
    });
}

#[test]
fn foreground_parent_loss_retires_only_its_own_serving_host() {
    runtime().block_on(async {
        let mut tree = ForegroundTree::new();
        let mut parent = admitted_parent(&tree, "serving").await;
        let host = foreground_process(&tree);
        let fixture = tree.fixture();
        let deadline = Instant::now() + BUDGET;
        let metadata = loop {
            if let Ok(view) = fixture.layout.discovery_view(false)
                && let Some(candidate) = view
                    .observe_ready(
                        &fixture.project,
                        fixture.layout.endpoint_directory().to_owned(),
                    )
                    .unwrap()
                && connect_control(candidate.metadata()).await.is_ok()
            {
                break candidate.metadata().clone();
            }
            assert!(
                Instant::now() < deadline,
                "foreground host was not published"
            );
            sleep(Duration::from_millis(10)).await;
        };
        let unrelated = Fixture::new();
        let (mut unrelated_process, unrelated_metadata) = unrelated.start().await;
        publish_marker(fixture.root.path(), "exit-parent", b"go");
        wait_child_exit(&mut parent).await;
        wait_pinned_exit(&host).await;
        assert_eq!(pinned_exit_code(&host), 0);
        assert!(
            !fixture
                .layout
                .endpoint_directory()
                .join("endpoint.json")
                .exists()
        );
        assert!(connect_control(&metadata).await.is_err());
        assert!(unrelated_process.0.try_wait().unwrap().is_none());
        assert!(connect_control(&unrelated_metadata).await.is_ok());
        let stopped = force_shutdown_host(&unrelated_metadata).await.unwrap();
        await_host_stopped(&stopped).await.unwrap();
        unrelated.exited(&mut unrelated_process).await;
        tree.shutdown().unwrap();
    });
}

#[test]
fn detached_host_survives_its_short_lived_launching_parent() {
    runtime().block_on(async {
        let mut tree = ForegroundTree::new();
        let mut parent = admitted_parent(&tree, "detached").await;
        let host = foreground_process(&tree);
        let fixture = tree.fixture();
        let deadline = Instant::now() + BUDGET;
        let metadata = loop {
            if let Ok(view) = fixture.layout.discovery_view(false)
                && let Some(candidate) = view
                    .observe_ready(
                        &fixture.project,
                        fixture.layout.endpoint_directory().to_owned(),
                    )
                    .unwrap()
                && connect_control(candidate.metadata()).await.is_ok()
            {
                break candidate.metadata().clone();
            }
            assert!(
                Instant::now() < deadline,
                "detached fixture host was not published"
            );
            sleep(Duration::from_millis(10)).await;
        };
        publish_marker(fixture.root.path(), "exit-parent", b"go");
        wait_child_exit(&mut parent).await;
        assert!(host.is_alive().unwrap());
        assert!(connect_control(&metadata).await.is_ok());
        let stopped = force_shutdown_host(&metadata).await.unwrap();
        await_host_stopped(&stopped).await.unwrap();
        wait_pinned_exit(&host).await;
        assert_eq!(pinned_exit_code(&host), 0);
        tree.shutdown().unwrap();
    });
}
