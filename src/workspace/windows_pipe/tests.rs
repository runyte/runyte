// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{EndpointLocation, RegistrySet},
        windows_process_identity::ProcessIdentity,
    },
};
use std::future::{Future, poll_fn};
use std::{
    fs,
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{CREATE_NO_WINDOW, WaitForSingleObject},
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn fixture() -> (TestRuntimeRoot, EndpointLocation) {
    let root = TestRuntimeRoot::new("native-host-pipe").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let registries = RegistrySet::open(&[root.join("registry")]).unwrap();
    let endpoint = EndpointLocation::new(&project, root.join("endpoint"), registries).unwrap();
    (root, endpoint)
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

async fn pair(
    listener: &mut Listener,
) -> (Connection<NamedPipeServer>, Connection<NamedPipeClient>) {
    // Poll both sides as real independent local peers do; the client must not
    // require the server's accept future to remain unpolled until open returns.
    let metadata = listener.metadata().clone();
    let end = deadline();
    let (server, client) = tokio::join!(listener.accept(end), connect(&metadata, end));
    (server.unwrap(), client.unwrap())
}

async fn assert_name_released(address: &PipeAddress) {
    // Mio owns completion references after the Rust stream drops. Observe the
    // actual namespace release while the runtime dispatches IOCP cancellation;
    // a synchronous rebind in the same task cannot establish handle cleanup.
    timeout_at(deadline(), async {
        loop {
            match instance(address, true) {
                Ok(rebound) => return drop(rebound),
                Err(error) => assert_eq!(
                    error.raw_os_error(),
                    Some(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32)
                ),
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("native pipe name was retained after all owners dropped");
}

async fn poll_pending<F: Future>(future: &mut Pin<Box<F>>) {
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

fn unread_native_bytes(pipe: &impl AsRawHandle) -> u32 {
    let mut available = 0;
    let success = unsafe {
        windows_sys::Win32::System::Pipes::PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(success, 0, "{}", io::Error::last_os_error());
    available
}

async fn fill_unread_pipe<S: AsyncWrite + AsRawHandle + Unpin>(
    writer: &mut Connection<S>,
    reader: &impl AsRawHandle,
) {
    let payload = [b'x'; WRITE_CHUNK_BYTES];
    let writing = async {
        // No peer reads occur. More than one MiB accepted would mean this
        // bounded native pipe no longer provides the expected backpressure.
        let mut accepted = 0;
        loop {
            let count = writer.write(&payload).await.unwrap();
            assert!(count > 0 && count <= WRITE_CHUNK_BYTES);
            accepted += count;
            assert!(accepted <= 1024 * 1024, "unbounded native write acceptance");
            writer.flush().await.unwrap();
        }
    };
    let filled = async {
        loop {
            if unread_native_bytes(reader) >= PIPE_BUFFER_BYTES {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    timeout_at(deadline(), async {
        tokio::select! {
            () = writing => unreachable!("bounded writer never finishes"),
            () = filled => {}
        }
    })
    .await
    .expect("fixture never filled the unread native pipe quota");

    // The last write normally straddles the full quota. If it ended exactly at
    // that boundary, queue one further bounded write to establish backpressure.
    let flushed = poll_fn(|cx| Poll::Ready(Pin::new(&mut *writer).poll_flush(cx))).await;
    match flushed {
        Poll::Pending => {}
        Poll::Ready(result) => {
            result.unwrap();
            let count = timeout_at(deadline(), writer.write(&payload))
                .await
                .unwrap()
                .unwrap();
            assert!(count > 0 && count <= WRITE_CHUNK_BYTES);
        }
    }
    let mut flushing = Box::pin(writer.flush());
    poll_pending(&mut flushing).await;
}

#[test]
fn writes_are_chunk_bounded_and_flush_preserves_exact_payload_in_both_directions() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let (mut server, mut client) = pair(&mut listener).await;
        let payload: Vec<u8> = (0..WRITE_CHUNK_BYTES * 2 + 17)
            .map(|index| (index % 251) as u8)
            .collect();
        for client_writes in [true, false] {
            let mut received = vec![0; payload.len()];
            if client_writes {
                let count = timeout_at(deadline(), client.write(&payload))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(count > 0 && count <= WRITE_CHUNK_BYTES);
                timeout_at(deadline(), client.write_all(&payload[count..]))
                    .await
                    .unwrap()
                    .unwrap();
                timeout_at(deadline(), client.flush())
                    .await
                    .unwrap()
                    .unwrap();
                timeout_at(deadline(), server.read_exact(&mut received))
                    .await
                    .unwrap()
                    .unwrap();
            } else {
                let count = timeout_at(deadline(), server.write(&payload))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(count > 0 && count <= WRITE_CHUNK_BYTES);
                timeout_at(deadline(), server.write_all(&payload[count..]))
                    .await
                    .unwrap()
                    .unwrap();
                timeout_at(deadline(), server.shutdown())
                    .await
                    .unwrap()
                    .unwrap();
                timeout_at(deadline(), client.read_exact(&mut received))
                    .await
                    .unwrap()
                    .unwrap();
            }
            assert_eq!(received, payload);
        }
    });
}

#[test]
fn flush_waits_for_backpressured_native_writes_and_reports_peer_disconnection() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let (mut server, client) = pair(&mut listener).await;
        fill_unread_pipe(&mut server, &client.stream).await;
        // Forced peer disconnection cannot turn a pending payload into a
        // successful flush. Tokio's unwrapped no-op flush would miss this.
        drop(client);
        assert!(
            timeout_at(deadline(), server.flush())
                .await
                .unwrap()
                .is_err()
        );
    });
}

#[test]
fn first_instance_collision_refuses_readiness_without_displacing_existing_pipe() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let prepared = endpoint.prepare(None).unwrap();
        let address = prepared.metadata().address.clone();
        let existing = instance(&address, true).unwrap();
        assert!(Listener::bind(prepared).is_err());
        assert!(endpoint.read_ready().unwrap().is_none());
        assert!(instance(&address, true).is_err());
        let client = ClientOptions::new().open(address.as_str()).unwrap();
        timeout_at(deadline(), existing.connect())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            peer(&client, false).unwrap().identity(),
            ProcessIdentity::current().unwrap()
        );
        drop(client);
        drop(existing);
        assert_name_released(&address).await;
    });
}

#[test]
fn successive_accepts_keep_one_private_byte_instance_and_retained_peer_proofs() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        assert_eq!(
            endpoint.read_ready().unwrap().as_ref(),
            Some(listener.metadata())
        );
        let identity = ProcessIdentity::current().unwrap();
        for byte in 0..24 {
            // First-instance exclusion must hold even with no admitted peers.
            assert!(instance(&listener.metadata().address, true).is_err());
            let (mut server, mut client) = pair(&mut listener).await;
            assert_eq!(server.peer().identity(), identity);
            assert_eq!(client.peer().identity(), identity);
            assert!(server.peer().is_alive().unwrap());
            let info = listener.pending().unwrap().info().unwrap();
            assert_eq!(info.mode, PipeMode::Byte);
            assert_eq!(info.max_instances, MAX_INSTANCES as u32);
            assert_eq!(info.in_buffer_size, PIPE_BUFFER_BYTES);
            assert_eq!(info.out_buffer_size, PIPE_BUFFER_BYTES);
            timeout_at(deadline(), client.write_all(&[byte]))
                .await
                .unwrap()
                .unwrap();
            let mut received = [255];
            timeout_at(deadline(), server.read_exact(&mut received))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, [byte]);
            timeout_at(deadline(), server.write_all(&[byte ^ 255]))
                .await
                .unwrap()
                .unwrap();
            timeout_at(deadline(), client.read_exact(&mut received))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, [byte ^ 255]);
        }
        assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
    });
}

#[test]
fn actual_server_creation_identity_must_match_published_metadata() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        for wrong_pid in [false, true] {
            let prepared = endpoint.prepare(None).unwrap();
            let server = instance(&prepared.metadata().address, true).unwrap();
            let mut metadata = prepared.metadata().clone();
            if wrong_pid {
                metadata.process.pid = metadata.process.pid.wrapping_add(1).max(1);
            } else {
                metadata.process.creation_time ^= 1;
            }
            assert_eq!(
                connect(&metadata, deadline()).await.unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            drop(server);
            drop(prepared);
        }
        let prepared = endpoint.prepare(None).unwrap();
        let mut metadata = prepared.metadata().clone();
        metadata.protocol = 0;
        assert_eq!(
            connect(&metadata, deadline()).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    });
}

#[test]
fn cancelled_and_timed_out_accept_retains_pending_for_the_next_client() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let pending = listener.pending().unwrap().as_raw_handle();
        let mut operation = Box::pin(listener.accept(deadline()));
        poll_pending(&mut operation).await;
        drop(operation);
        assert!(listener.is_usable());
        assert_eq!(listener.pending().unwrap().as_raw_handle(), pending);
        assert_eq!(
            listener
                .accept(Instant::now() + Duration::from_millis(10))
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(listener.is_usable());
        assert_eq!(listener.pending().unwrap().as_raw_handle(), pending);
        let peers = pair(&mut listener).await;
        drop(peers);
        assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
    });
}

#[test]
fn cancelled_missing_and_busy_connects_release_without_background_workers() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let prepared = endpoint.prepare(None).unwrap();
        let metadata = prepared.metadata().clone();
        let mut missing = Box::pin(connect(&metadata, deadline()));
        poll_pending(&mut missing).await;
        drop(missing);
        assert_eq!(
            connect(&metadata, Instant::now() + Duration::from_millis(10))
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        let mut listener = Listener::bind(prepared).unwrap();
        let first = connect(listener.metadata(), deadline()).await.unwrap();
        let mut busy = Box::pin(connect(&metadata, deadline()));
        poll_pending(&mut busy).await;
        drop(busy);
        let server = listener.accept(deadline()).await.unwrap();
        drop((first, server));
        drop(pair(&mut listener).await);
        assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
    });
}

#[test]
fn returned_connections_are_caller_owned_and_outlive_listener_drop() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let metadata = listener.metadata().clone();
        let (mut server, mut client) = pair(&mut listener).await;
        let proof = server.peer().clone();
        drop(listener);
        assert!(endpoint.read_ready().unwrap().is_none());
        timeout_at(deadline(), client.write_all(b"retained"))
            .await
            .unwrap()
            .unwrap();
        let mut bytes = [0; 8];
        timeout_at(deadline(), server.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes, b"retained");
        assert!(proof.is_alive().unwrap());
        assert!(instance(&metadata.address, true).is_err());
        drop((server, client));
        assert_name_released(&metadata.address).await;
        assert_eq!(proof.identity(), ProcessIdentity::current().unwrap());
    });
}

struct FixtureChild(Child);
impl Drop for FixtureChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        unsafe {
            WaitForSingleObject(self.0.as_raw_handle(), 5000);
        }
    }
}

#[test]
fn accepted_pipe_pins_the_actual_compiled_client_process_through_exit() {
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let record = root.join("fixture-metadata.json");
        fs::write(&record, serde_json::to_vec(listener.metadata()).unwrap()).unwrap();
        let mut child = FixtureChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "workspace::windows_pipe::tests::client_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env("RUNYTE_NATIVE_PIPE_FIXTURE_METADATA", &record)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_CACHE_HOME", root.join("cache"))
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut server = listener
            .accept(Instant::now() + Duration::from_secs(10))
            .await
            .unwrap();
        let proof = server.peer().clone();
        assert_eq!(proof.identity().pid, child.0.id());
        assert_ne!(proof.identity().pid, std::process::id());
        let mut bytes = [0; 12];
        timeout_at(deadline(), server.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            proof.identity().pid
        );
        assert_eq!(
            u64::from_le_bytes(bytes[4..].try_into().unwrap()),
            proof.identity().creation_time
        );
        timeout_at(deadline(), server.write_all(b"q"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(child.0.as_raw_handle(), 10000) },
            WAIT_OBJECT_0
        );
        assert!(child.0.try_wait().unwrap().unwrap().success());
        assert!(!proof.is_alive().unwrap());
        assert_eq!(server.peer().identity(), proof.identity());
    });
}

#[test]
#[ignore = "compiled native pipe client for retained peer identity acceptance"]
fn client_fixture() {
    let record = std::env::var_os("RUNYTE_NATIVE_PIPE_FIXTURE_METADATA").unwrap();
    let metadata = EndpointMetadata::from_json(&fs::read(record).unwrap()).unwrap();
    runtime().block_on(async {
        let mut client = connect(&metadata, deadline()).await.unwrap();
        assert_eq!(client.peer().identity(), metadata.process);
        let own = ProcessIdentity::current().unwrap();
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&own.pid.to_le_bytes());
        bytes.extend_from_slice(&own.creation_time.to_le_bytes());
        timeout_at(deadline(), client.write_all(&bytes))
            .await
            .unwrap()
            .unwrap();
        let mut byte = [0];
        timeout_at(deadline(), client.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(byte, [b'q']);
    });
}
#[test]
fn exhausted_admission_preserves_waiting_client_until_capacity_recovers() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let mut peers = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            peers.push(pair(&mut listener).await);
        }
        let pending = listener.pending().unwrap().as_raw_handle();
        let mut waiting = connect(listener.metadata(), deadline()).await.unwrap();
        waiting.write_all(b"waiting client").await.unwrap();
        assert_eq!(
            listener.accept(deadline()).await.unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(listener.pending().unwrap().as_raw_handle(), pending);
        assert!(listener.retiring.is_none());
        assert_eq!(listener.slots.available_permits(), 0);
        assert!(
            instance(&listener.metadata().address, false).is_err(),
            "created an eighteenth instance"
        );
        drop(peers.pop().unwrap());
        let mut recovered = listener.accept(deadline()).await.unwrap();
        let mut bytes = [0; 14];
        timeout_at(deadline(), recovered.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes, b"waiting client");
        assert_eq!(listener.slots.available_permits(), 0);
        drop((recovered, waiting));
        drop(pair(&mut listener).await);
        drop(peers);
        assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
    });
}

#[test]
fn replacement_failure_closes_refused_stream_and_permanently_refuses_accept() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let mut client = connect(listener.metadata(), deadline()).await.unwrap();
        timeout_at(deadline(), listener.pending().unwrap().connect())
            .await
            .unwrap()
            .unwrap();
        listener.retiring = Some(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fixture refusal",
        ));
        let error = listener
            .accept_with(
                deadline(),
                |_| Err(io::Error::other("injected replacement failure")),
                Instant::now,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected replacement failure"));
        assert!(!listener.is_usable());
        for _ in 0..2 {
            assert_eq!(
                listener.accept(deadline()).await.unwrap_err().kind(),
                io::ErrorKind::NotConnected
            );
        }
        assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
        let mut byte = [0];
        let closed = timeout_at(deadline(), client.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(closed, Ok(0)) || closed.is_err());
    });
}

#[test]
fn cancelled_retirement_preserves_refusal_and_replacement_discards_prefetched_bytes() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let mut refused = connect(listener.metadata(), deadline()).await.unwrap();
        timeout_at(deadline(), listener.pending().unwrap().connect())
            .await
            .unwrap()
            .unwrap();
        timeout_at(deadline(), refused.write_all(b"PREDECESSOR BYTES"))
            .await
            .unwrap()
            .unwrap();
        timeout_at(deadline(), refused.flush())
            .await
            .unwrap()
            .unwrap();
        // Readiness dispatches Mio's read-ahead completion without consuming its
        // cached payload. This is the state that DisconnectNamedPipe cannot reset.
        timeout_at(deadline(), listener.pending().unwrap().readable())
            .await
            .unwrap()
            .unwrap();
        let pending = listener.pending().unwrap().as_raw_handle();
        listener.retiring = Some(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "recorded refusal",
        ));
        let mut operation = Box::pin(listener.accept_with(
            deadline(),
            |_| Err(io::Error::from_raw_os_error(ERROR_PIPE_BUSY as i32)),
            Instant::now,
        ));
        poll_pending(&mut operation).await;
        drop(operation);
        assert!(listener.is_usable());
        assert_eq!(listener.pending().unwrap().as_raw_handle(), pending);
        assert_eq!(
            listener.retiring.as_ref().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
        let error = listener
            .accept_with(
                Instant::now() + Duration::from_millis(10),
                |_| Err(io::Error::from_raw_os_error(ERROR_PIPE_BUSY as i32)),
                Instant::now,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(listener.is_usable());
        assert_eq!(
            listener.retiring.as_ref().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
        let refusal = listener.accept(deadline()).await.unwrap_err();
        assert_eq!(refusal.kind(), io::ErrorKind::PermissionDenied);
        assert!(refusal.to_string().contains("recorded refusal"));
        assert!(listener.retiring.is_none());
        assert_ne!(listener.pending().unwrap().as_raw_handle(), pending);
        let (mut server, mut client) = pair(&mut listener).await;
        timeout_at(deadline(), client.write_all(b"NEW CLIENT"))
            .await
            .unwrap()
            .unwrap();
        let mut bytes = [0; 10];
        timeout_at(deadline(), server.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes, b"NEW CLIENT");
    });
}

#[test]
fn deadline_at_connect_or_replacement_boundary_never_admits_a_late_connection() {
    use std::cell::Cell;
    runtime().block_on(async {
        for expire_after_connect in [true, false] {
            let (_root, endpoint) = fixture();
            let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
            let mut client = connect(listener.metadata(), deadline()).await.unwrap();
            let pending = listener.pending().unwrap().as_raw_handle();
            let end = deadline();
            let initial = Instant::now();
            let clock_reads = Cell::new(0);
            let created = Cell::new(false);
            let error = listener
                .accept_with(
                    end,
                    |address| {
                        let replacement = instance(address, false)?;
                        created.set(true);
                        Ok(replacement)
                    },
                    || {
                        clock_reads.set(clock_reads.get() + 1);
                        if (expire_after_connect && clock_reads.get() >= 2) || created.get() {
                            end
                        } else {
                            initial
                        }
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert_eq!(created.get(), !expire_after_connect);
            assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
            if expire_after_connect {
                assert_eq!(listener.pending().unwrap().as_raw_handle(), pending);
                assert!(listener.retiring.is_some());
                // A fresh caller budget finishes retirement and returns the
                // original refusal once, without admitting the expired peer.
                assert_eq!(
                    listener.accept(deadline()).await.unwrap_err().kind(),
                    io::ErrorKind::TimedOut
                );
            }
            assert!(listener.retiring.is_none());
            assert_ne!(listener.pending().unwrap().as_raw_handle(), pending);
            let mut byte = [0];
            let closed = timeout_at(deadline(), client.read(&mut byte))
                .await
                .unwrap();
            assert!(matches!(closed, Ok(0)) || closed.is_err());
            drop(client);
            drop(pair(&mut listener).await);
            assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
        }
    });
}

#[test]
fn cancelled_healthy_replacement_wait_preserves_connected_peer_for_next_accept() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let mut client = connect(listener.metadata(), deadline()).await.unwrap();
        client.write_all(b"same peer").await.unwrap();
        let pending = listener.pending().unwrap().as_raw_handle();
        let mut operation = Box::pin(listener.accept_with(
            deadline(),
            |_| Err(io::Error::from_raw_os_error(ERROR_PIPE_BUSY as i32)),
            Instant::now,
        ));
        poll_pending(&mut operation).await;
        drop(operation);
        assert!(listener.is_usable());
        assert!(listener.retiring.is_none());
        assert_eq!(listener.pending().unwrap().as_raw_handle(), pending);
        assert_eq!(listener.slots.available_permits(), MAX_CONNECTIONS);
        let mut accepted = listener.accept(deadline()).await.unwrap();
        let mut bytes = [0; 9];
        timeout_at(deadline(), accepted.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes, b"same peer");
    });
}
#[test]
fn dropping_backpressured_writer_releases_native_handle_without_peer_reads() {
    // The closure witness probes an old numeric handle. Isolate it from other
    // parallel tests that could create a handle reusing that number after close.
    let root = TestRuntimeRoot::new("native-pipe-write-drop").unwrap();
    let mut child = FixtureChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "workspace::windows_pipe::tests::blocked_writer_drop_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("RUST_BACKTRACE", "0")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    assert_eq!(
        unsafe { WaitForSingleObject(child.0.as_raw_handle(), 15000) },
        WAIT_OBJECT_0
    );
    assert!(child.0.try_wait().unwrap().unwrap().success());
}

#[test]
#[ignore = "isolated compiled fixture for native cancelled-write handle release"]
fn blocked_writer_drop_fixture() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let metadata = listener.metadata().clone();
        let (server, mut client) = pair(&mut listener).await;
        fill_unread_pipe(&mut client, &server.stream).await;
        let handle = client.stream.as_raw_handle();
        let mut flags = 0;
        assert_ne!(
            unsafe { windows_sys::Win32::Foundation::GetHandleInformation(handle, &mut flags) },
            0
        );
        drop(client);
        // No native handles are created between capture and this observation.
        // Timer/IOCP progress releases cancellation-owned Mio references, while
        // the still-live server never reads payload to unblock the queued write.
        timeout_at(deadline(), async {
            loop {
                if unsafe {
                    windows_sys::Win32::Foundation::GetHandleInformation(handle, &mut flags)
                } == 0
                {
                    assert_eq!(
                        io::Error::last_os_error().raw_os_error(),
                        Some(windows_sys::Win32::Foundation::ERROR_INVALID_HANDLE as i32)
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("dropped writer retained its native handle behind unread data");
        drop(server);
        drop(listener);
        assert_name_released(&metadata.address).await;
    });
}
