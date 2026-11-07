// SPDX-License-Identifier: MPL-2.0

use super::*;

struct WorkerProgress<'a> {
    sandbox: &'a TestSandbox,
    root: &'a Path,
}

impl WorkerProgress<'_> {
    fn diagnostics(&self, child: &mut ChildGuard) -> String {
        use std::io::Read;

        let host = child.0.as_mut().map(|child| child.try_wait());
        let worker = fs::read_to_string(self.sandbox.runtime.join("application-worker.starts"))
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
            .map(process_is_running);
        let mut report = format!("host exit={host:?}; worker running={worker:?}");
        for name in [
            "application-worker.trace",
            "application-worker.stderr",
            "host.stderr",
            "host.input",
        ] {
            let mut bytes = Vec::new();
            if let Ok(file) = fs::File::open(self.sandbox.runtime.join(name)) {
                let _ = file.take(8192).read_to_end(&mut bytes);
            }
            report.push_str(&format!("\n{name}: {}", String::from_utf8_lossy(&bytes)));
        }
        // Diagnostics contain only this fixture's input and protocol exchange.
        // Bound each capture and remove paths and their derived workspace IDs.
        let id = runyte::workspace::workspace_id(self.root);
        report
            .replace(&id, "<workspace-id>")
            .replace(&id[..ABBREVIATED_WORKSPACE_ID], "<workspace-id>")
            .replace(&*self.root.to_string_lossy(), "<project>")
            .replace(&*self.sandbox.runtime_dir().to_string_lossy(), "<runtime>")
            .replace(env!("CARGO_MANIFEST_DIR"), "<checkout>")
    }

    async fn marker(
        &self,
        suffix: &str,
        label: &str,
        child: &mut ChildGuard,
        mut client: Option<&mut LocalClient>,
    ) -> usize {
        let path = self
            .sandbox
            .runtime
            .join(format!("application-worker.{suffix}"));
        let started = Instant::now();
        let mut frames = 0;
        while !path.exists() {
            assert!(
                started.elapsed() < HOST_RESPONSE_TIMEOUT,
                "stable plugin worker did not reach {label} after {:?}; drained {frames} frames; {}",
                started.elapsed(),
                self.diagnostics(child)
            );
            // A frame is not an input acknowledgement. Keep reading while the
            // worker advances: otherwise an earlier asynchronous redraw can
            // block the connection writer before it reads our F12 request.
            tokio::select! {
                result = async {
                    match client.as_deref_mut() {
                        Some(client) => client.recv().await,
                        None => std::future::pending().await,
                    }
                } => match result {
                    Ok(Some(HostResponse::Frame { .. } | HostResponse::EditorDamage { .. } | HostResponse::TerminalDamage { .. })) => frames += 1,
                    other => panic!("host stopped advancing {label}: {other:?}; {}", self.diagnostics(child)),
                },
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
        eprintln!(
            "stable plugin {label}: {:?}, drained {frames} frames",
            started.elapsed()
        );
        frames
    }
}

async fn jobs(client: &mut LocalClient, expected: usize, attached: bool) {
    client.send(&ClientRequest::Health).await.unwrap();
    match semantic_response_after(client, None, "checking stable plugin protected work").await {
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
        other => panic!("expected stable plugin health, got {other:?}"),
    }
}

#[tokio::test]
async fn stable_view_and_finite_job_survive_repeated_real_attachments() {
    stable_lifecycle(false).await;
}

#[tokio::test]
async fn stable_lifecycle_drains_queued_frames_before_command_delivery() {
    stable_lifecycle(true).await;
}

async fn stable_lifecycle(queued_frames: bool) {
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
exec 2> "$0.stderr"
trace_count=0
trace() {
    if [ "$trace_count" -lt 32 ]; then
        printf '%.256s\n' "$1" >> "$0.trace"
        trace_count=$((trace_count + 1))
    fi
}
trap 'trace "worker exit=$?"' 0
accepted() {
    trace "$1 response"
    case "$message" in *'"error"'*) trace "$message"; exit 2;; esac
}
printf '%s\n' "$$" >> "$0.starts"
read -r hello
trace "hello received; register sent"
printf '%s\n' '{"type":"register","version":"runyte-1","runyte":">=0.0.0, <18446744073709551615.0.0","required_features":[],"optional_features":[],"name":"Attachment application","commands":[{"name":"open","description":"Open retained progress","context":"workspace"}],"required_capabilities":["views","jobs"],"optional_capabilities":[]}'
read -r registered
trace "registered"
printf 'ready\n' > "$0.ready"
while read -r message; do
    case "$message" in
        *'"method":"command.invoke"'*)
            trace "command.invoke received; view.create sent"
            invocation=$(printf '%s' "$message" | sed -n 's/.*"id":"\(h:[0-9]*\)".*/\1/p')
            printf '%s\n' '{"type":"request","id":"p:1","method":"view.create","params":{"model":{"title":"Retained application","purpose":"list","rows":[{"id":"stable","text":"Waiting for detached completion","role":"ordinary"}]}}}'
            ;;
        *'"type":"response","id":"p:1"'*)
            accepted "view.create"
            view=$(printf '%s' "$message" | sed -n 's/.*"view":"\([^"]*\)".*/\1/p')
            revision=$(printf '%s' "$message" | sed -n 's/.*"revision":"\([^"]*\)".*/\1/p')
            printf '{"type":"request","id":"p:2","method":"pane.show","params":{"invocation":"%s","view":"%s"}}\n' "$invocation" "$view"
            ;;
        *'"type":"response","id":"p:2"'*)
            accepted "pane.show"
            printf '%s\n' '{"type":"request","id":"p:3","method":"job.create","params":{"title":"Detached application work","deadline_seconds":60}}'
            ;;
        *'"type":"response","id":"p:3"'*)
            accepted "job.create"
            job=$(printf '%s' "$message" | sed -n 's/.*"job":"\([^"]*\)".*/\1/p')
            printf '{"type":"response","id":"%s","result":{"job":"%s"}}\n' "$invocation" "$job"
            trace "invocation completed; finite job accepted"
            printf 'working\n' > "$0.working"
            while [ ! -e "$0.release" ]; do sleep 0.01; done
            printf '{"type":"request","id":"p:4","method":"view.publish","params":{"view":"%s","expected_revision":"%s","model":{"title":"Retained application","purpose":"list","rows":[{"id":"stable","text":"Completed while detached","role":"heading"}]}}}\n' "$view" "$revision"
            ;;
        *'"type":"response","id":"p:4"'*)
            accepted "view.publish"
            printf '{"type":"request","id":"p:5","method":"job.finish","params":{"job":"%s","state":"succeeded"}}\n' "$job"
            ;;
        *'"type":"response","id":"p:5"'*)
            accepted "job.finish"
            printf 'finished\n' > "$0.finished"
            ;;
    esac
done
"#).unwrap();
    fs::write(sandbox.cache.join("runyte/config.yaml"), format!(
        "lsp:\n  enable: false\nplugins:\n  - id: application\n    enabled: true\n    api: runyte-1\n    runyte: \">=0.0.0, <18446744073709551615.0.0\"\n    executable: {}\n    capabilities: [views, jobs]\n    bindings:\n      open: F12\n", program.display()
    )).unwrap();
    let child = sandbox
        .bundled_runyte()
        .args(["--serve", "note.txt"])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", sandbox.runtime_dir())
        .env("XDG_CACHE_HOME", sandbox.cache_dir())
        .env("RUNYTE_INPUT_TRACE", sandbox.runtime.join("host.input"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(sandbox.runtime.join("host.stderr")).unwrap())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_in_tree_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(sandbox.runtime_dir()),
    )
    .unwrap();
    let progress = WorkerProgress {
        sandbox: &sandbox,
        root: &root,
    };
    assert!(
        wait_for_endpoint(&mut child, &endpoint).await,
        "{}",
        progress.diagnostics(&mut child)
    );
    progress
        .marker("ready", "registration", &mut child, None)
        .await;
    let starts = fs::read_to_string(sandbox.runtime.join("application-worker.starts")).unwrap();
    let worker_pid: i32 = starts.trim().parse().unwrap();
    let (mut first, _) = connect_interactive_when_available(&endpoint, geometry()).await;
    let _ = response(&mut first).await;
    if queued_frames {
        // Reproduce an asynchronous redraw ahead of F12. Space a bounded burst
        // across render turns so visual-slot coalescing cannot erase the entire
        // backlog. This delays reads to provoke backpressure, not to await work.
        let mut large = geometry();
        large.screen.width = 240;
        large.screen.height = 80;
        large.editor.width = 240;
        large.editor.height = 78;
        large.status.width = 240;
        large.status.y = 78;
        large.message.width = 240;
        large.message.y = 79;
        first
            .send(&ClientRequest::Resize {
                geometry: large.into(),
            })
            .await
            .unwrap();
        for _ in 0..32 {
            first.send(&ClientRequest::Resynchronize).await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    let _ = send_input(&mut first, KeyStroke::parse("F12").unwrap()).await;
    progress
        .marker(
            "working",
            "accepted finite work",
            &mut child,
            Some(&mut first),
        )
        .await;
    let publish_after = Instant::now() + Duration::from_millis(110);
    jobs(&mut first, 1, true).await;
    detach(&mut first, "detaching active stable plugin work").await;

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
    progress
        .marker(
            "finished",
            "detached publication and completion",
            &mut child,
            None,
        )
        .await;
    jobs(&mut control, 0, false).await;
    wait_for_session_preview(
        &mut control,
        "stable plugin detached publication",
        |preview| {
            preview.panes.iter().any(|pane| {
                pane.lines
                    .iter()
                    .any(|line| line.contains("Completed while detached"))
            })
        },
    )
    .await;
    for _ in 0..2 {
        let (mut attached, _) = connect_interactive_when_available(&endpoint, geometry()).await;
        assert!(frame_text(&response(&mut attached).await).contains("Completed while detached"));
        jobs(&mut attached, 0, true).await;
        detach(
            &mut attached,
            "reattaching the same completed stable plugin view",
        )
        .await;
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
