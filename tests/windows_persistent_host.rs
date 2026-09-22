// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    app::FrameGeometry,
    protocol::{
        ClientRequest, FeatureGroup, HostResponse, TransportChange, WaitStatus, encode_path,
    },
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::EndpointMetadata,
        windows_lifecycle::{await_host_stopped, connect_control, force_shutdown_host},
        windows_location::{CapturedRoots, EXPECTED_LAYOUT_ENV, LocationInputs, ResolvedLayout},
        windows_transport::LocalClient,
    },
};
use std::{
    fs,
    io::Read,
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::time::{Instant, sleep, timeout};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{CREATE_NO_WINDOW, WaitForSingleObject},
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
                interactive_attached: false,
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
fn native_host_layout_mismatch_and_foreground_refuse_before_publication() {
    runtime().block_on(async {
        for foreground in [false, true] {
            let fixture = Fixture::new();
            let mut command = fixture.command();
            if foreground {
                // Build a fresh command so the dedicated-only flag is absent.
                command = Command::new(env!("CARGO_BIN_EXE_runyte"));
                command
                    .args(["--serve", "--project-root"])
                    .arg(&fixture.project)
                    .arg("--config")
                    .arg(&fixture.config)
                    .current_dir(&fixture.project)
                    .env("XDG_CONFIG_HOME", fixture.root.join("config"))
                    .env("XDG_CACHE_HOME", fixture.root.join("cache"))
                    .env("XDG_RUNTIME_DIR", fixture.root.join("runtime"))
                    .env("RUNYTE_ALL_HOSTS_DIR", fixture.root.join("inventory"))
                    .env("PATH", "")
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
            } else {
                command.env(EXPECTED_LAYOUT_ENV, "mismatched-layout");
            }
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
        }
    });
}
