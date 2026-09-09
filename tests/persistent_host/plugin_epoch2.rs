// SPDX-License-Identifier: MPL-2.0

use super::*;

async fn marker(path: &Path, label: &str) {
    let deadline = Instant::now() + HOST_RESPONSE_TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "epoch 2 worker did not reach {label}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn jobs(client: &mut LocalClient, expected: usize, attached: bool) {
    client.send(&ClientRequest::Health).await.unwrap();
    match semantic_response_after(client, None, "checking epoch 2 protected work").await {
        HostResponse::Health {
            plugin_jobs,
            activity_leases,
            interactive_attached,
            unsaved_buffers,
            ..
        } => {
            assert_eq!(plugin_jobs, expected);
            assert_eq!(activity_leases, 0);
            assert_eq!(interactive_attached, attached);
            assert_eq!(unsaved_buffers, 0);
        }
        other => panic!("expected epoch 2 health, got {other:?}"),
    }
}

#[tokio::test]
async fn epoch2_view_and_finite_job_survive_repeated_real_attachments() {
    let sandbox = TestSandbox::new();
    let root = project();
    let program = sandbox.runtime.join("application-worker");
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    // Only the checked-in stand-in executes. The adjacent behavior is data,
    // uses POSIX tools, and needs no Python/Node installation on Rust CI.
    fs::write(sandbox.runtime.join("application-worker.behavior"), r#"
printf '%s\n' "$$" >> "$0.starts"
read -r hello
printf '%s\n' '{"type":"register","version":"runyte-experimental-2","name":"Attachment application","commands":[{"name":"open","description":"Open retained progress","context":"workspace"}],"required_capabilities":["views","jobs"],"optional_capabilities":[]}'
read -r registered
printf 'ready\n' > "$0.ready"
while read -r message; do
    case "$message" in
        *'"method":"command.invoke"'*)
            invocation=$(printf '%s' "$message" | sed -n 's/.*"id":"\(h:[0-9]*\)".*/\1/p')
            printf '%s\n' '{"type":"request","id":"p:1","method":"view.create","params":{"model":{"title":"Retained epoch two","purpose":"list","rows":[{"id":"stable","text":"Waiting for detached completion","role":"ordinary"}]}}}'
            ;;
        *'"type":"response","id":"p:1"'*)
            view=$(printf '%s' "$message" | sed -n 's/.*"view":"\([^"]*\)".*/\1/p')
            revision=$(printf '%s' "$message" | sed -n 's/.*"revision":"\([^"]*\)".*/\1/p')
            printf '{"type":"request","id":"p:2","method":"pane.show","params":{"invocation":"%s","view":"%s"}}\n' "$invocation" "$view"
            ;;
        *'"type":"response","id":"p:2"'*)
            printf '%s\n' '{"type":"request","id":"p:3","method":"job.create","params":{"title":"Detached application work","deadline_seconds":60}}'
            ;;
        *'"type":"response","id":"p:3"'*)
            job=$(printf '%s' "$message" | sed -n 's/.*"job":"\([^"]*\)".*/\1/p')
            printf '{"type":"response","id":"%s","result":{"job":"%s"}}\n' "$invocation" "$job"
            printf 'working\n' > "$0.working"
            while [ ! -e "$0.release" ]; do sleep 0.01; done
            printf '{"type":"request","id":"p:4","method":"view.publish","params":{"view":"%s","expected_revision":"%s","model":{"title":"Retained epoch two","purpose":"list","rows":[{"id":"stable","text":"Completed while detached","role":"heading"}]}}}\n' "$view" "$revision"
            ;;
        *'"type":"response","id":"p:4"'*)
            case "$message" in *'"error"'*) exit 2;; esac
            printf '{"type":"request","id":"p:5","method":"job.finish","params":{"job":"%s","state":"succeeded"}}\n' "$job"
            ;;
        *'"type":"response","id":"p:5"'*)
            case "$message" in *'"error"'*) exit 3;; esac
            printf 'finished\n' > "$0.finished"
            ;;
    esac
done
"#).unwrap();
    fs::write(sandbox.cache.join("runyte/config.yaml"), format!(
        "lsp:\n  enable: false\nplugins:\n  - id: application\n    enabled: true\n    api: runyte-experimental-2\n    executable: {}\n    capabilities: [views, jobs]\n    bindings:\n      open: F12\n", program.display()
    )).unwrap();
    let child = sandbox
        .bundled_runyte()
        .args(["--serve", "note.txt"])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", sandbox.runtime_dir())
        .env("XDG_CACHE_HOME", sandbox.cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(sandbox.runtime_dir()),
    )
    .unwrap();
    assert!(wait_for_endpoint(&mut child, &endpoint).await);
    marker(
        &sandbox.runtime.join("application-worker.ready"),
        "registration",
    )
    .await;
    let starts = fs::read_to_string(sandbox.runtime.join("application-worker.starts")).unwrap();
    let worker_pid: i32 = starts.trim().parse().unwrap();
    let (mut first, _) = connect_interactive_when_available(&endpoint, geometry()).await;
    let _ = response(&mut first).await;
    let _ = send_input(&mut first, KeyStroke::parse("F12").unwrap()).await;
    marker(
        &sandbox.runtime.join("application-worker.working"),
        "accepted finite work",
    )
    .await;
    let publish_after = Instant::now() + Duration::from_millis(110);
    jobs(&mut first, 1, true).await;
    detach(&mut first, "detaching active epoch 2 work").await;

    let mut control = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Welcome { .. }
    ));
    jobs(&mut control, 1, false).await;
    let (mut second, _) = connect_interactive_when_available(&endpoint, geometry()).await;
    assert!(frame_text(&response(&mut second).await).contains("Waiting for detached completion"));
    jobs(&mut second, 1, true).await;
    detach(&mut second, "detaching the retained running view again").await;
    // Respect the public 10 Hz publication budget even on a very fast host.
    tokio::time::sleep(publish_after.saturating_duration_since(Instant::now())).await;
    fs::write(
        sandbox.runtime.join("application-worker.release"),
        "continue",
    )
    .unwrap();
    marker(
        &sandbox.runtime.join("application-worker.finished"),
        "detached publication and completion",
    )
    .await;
    jobs(&mut control, 0, false).await;
    wait_for_session_preview(&mut control, "epoch 2 detached publication", |preview| {
        preview.panes.iter().any(|pane| {
            pane.lines
                .iter()
                .any(|line| line.contains("Completed while detached"))
        })
    })
    .await;
    for _ in 0..2 {
        let (mut attached, _) = connect_interactive_when_available(&endpoint, geometry()).await;
        assert!(frame_text(&response(&mut attached).await).contains("Completed while detached"));
        jobs(&mut attached, 0, true).await;
        detach(&mut attached, "reattaching the same completed epoch 2 view").await;
    }
    assert_eq!(
        fs::read_to_string(sandbox.runtime.join("application-worker.starts")).unwrap(),
        starts
    );
    // Quiescent registration and retained readable models are not protection:
    // ordinary shutdown succeeds after the finite job has settled.
    shutdown(&mut control, ClientRequest::Shutdown).await;
    let status = tokio::task::spawn_blocking(move || child.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        unsafe { libc::kill(worker_pid, 0) },
        -1,
        "plugin child survived host shutdown"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    assert_eq!(fs::read_to_string(root.join("note.txt")).unwrap(), "base\n");
    fs::remove_dir_all(root).unwrap();
}
