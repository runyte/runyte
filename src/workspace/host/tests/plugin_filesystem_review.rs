// SPDX-License-Identifier: MPL-2.0

use super::filesystem::{applied_event, finished, local_service, response};
use super::filesystem_apply::{accept, confirmation};
use super::*;
use std::sync::{Mutex, atomic::Ordering};

fn expire(host: &mut WorkspaceHost, token: &str) {
    if let Some(deadline) = host
        .app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .get_mut(token)
    {
        *deadline = std::time::Instant::now();
    }
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline {
            token: token.into(),
        }),
    });
}

#[tokio::test]
async fn full_outbound_queue_at_filesystem_acceptance_cleans_unannounced_job() {
    let (root, mut host) = host();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    confirmation(&mut host, &mut events, &mut output, "create").await;
    let sender = &host.app.plugins.instances[&0].sender;
    while sender
        .try_send(HostMessage::Deadline {
            token: "queue-filler".into(),
            after_ms: None,
        })
        .is_ok()
    {}
    host.app
        .handle_key(crate::input::KeyStroke::parse("Enter").unwrap())
        .unwrap();
    host.sync_plugin_observers();
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert_eq!(host.protected_state().plugin_jobs, 0);
    assert!(host.filesystem_apply.is_none());
    assert!(!host.app.plugins.filesystem_applying);
    assert!(host.app.fs_confirmation.is_none());
    assert!(host.app.plugins.filesystem_confirmation.is_none());
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(!root.path().join("created.txt").exists());
    assert!(events.try_recv().is_err());
}

#[test]
fn queued_filesystem_job_can_be_cancelled_without_mutating_disk() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (root, mut host) = host();
        let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
        next(&mut output);
        let mut events = local_service(&mut host);
        confirmation(&mut host, &mut events, &mut output, "create").await;

        // Occupy the only blocking thread after preparation so acceptance
        // queues real apply work without relying on scheduler timing.
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = released.recv();
        });
        ready.await.unwrap();
        let (_, token) = accept(&mut host, &mut output);
        assert_eq!(
            host.filesystem_apply
                .as_ref()
                .unwrap()
                .phase
                .load(Ordering::SeqCst),
            0
        );
        request(
            &mut host,
            0,
            4,
            api::Request::JobCancel { job: token.clone() },
        );
        assert_eq!(job(&mut output).state, api::JobState::Cancelling);
        assert!(matches!(
            next(&mut output),
            api::HostMessage::Event {
                event: "job.changed",
                ..
            }
        ));
        assert_eq!(
            host.filesystem_apply
                .as_ref()
                .unwrap()
                .phase
                .load(Ordering::SeqCst),
            2
        );
        request(
            &mut host,
            0,
            5,
            api::Request::JobCancel { job: token.clone() },
        );
        assert_eq!(job(&mut output).state, api::JobState::Cancelling);
        expire(&mut host, &token);
        expire(&mut host, &token);
        assert!(host.app.plugins.instances.contains_key(&0));
        assert_eq!(host.protected_state().plugin_jobs, 1);
        assert!(!root.path().join("created.txt").exists());

        release.send(()).unwrap();
        blocker.await.unwrap();
        host.handle_plugin_event(applied_event(&mut events).await);
        assert_eq!(finished(&mut output).state, "cancelled");
        assert!(!root.path().join("created.txt").exists());
        assert_eq!(host.protected_state().plugin_jobs, 0);
        assert!(!host.app.plugins.filesystem_applying);
        // Only the original directory snapshot remains retained.
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            crate::plugin::filesystem::DIRECTORY_CHARGE
        );
    });
}

struct GatedTrash {
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    released: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl crate::fs_plan::TrashBackend for GatedTrash {
    fn delete(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        self.released.lock().unwrap().recv()?;
        std::fs::remove_file(path)?;
        Ok(())
    }
}

#[tokio::test]
async fn running_and_committed_filesystem_jobs_reject_cancellation_without_killing_owner() {
    let (root, mut host) = host();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "source").unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    host.app.set_trash_backend(Box::new(GatedTrash {
        entered: Mutex::new(Some(entered)),
        released: Mutex::new(released),
    }));
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    confirmation(&mut host, &mut events, &mut output, "trash").await;
    let (_, token) = accept(&mut host, &mut output);
    tokio::time::timeout(std::time::Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        host.filesystem_apply
            .as_ref()
            .unwrap()
            .phase
            .load(Ordering::SeqCst),
        1
    );
    request(
        &mut host,
        0,
        4,
        api::Request::JobCancel { job: token.clone() },
    );
    assert_eq!(
        response(&mut output).unwrap_err().code,
        api::ErrorCode::Conflict
    );
    expire(&mut host, &token);
    expire(&mut host, &token);
    assert!(host.app.plugins.instances.contains_key(&0));
    assert_eq!(host.protected_state().plugin_jobs, 1);
    assert!(
        !host
            .filesystem_apply
            .as_ref()
            .unwrap()
            .cancelled
            .load(Ordering::SeqCst)
    );
    assert!(source.exists());
    release.send(()).unwrap();
    let event = applied_event(&mut events).await;
    assert_eq!(
        host.filesystem_apply
            .as_ref()
            .unwrap()
            .phase
            .load(Ordering::SeqCst),
        3
    );
    assert!(!source.exists());
    request(
        &mut host,
        0,
        5,
        api::Request::JobCancel { job: token.clone() },
    );
    assert_eq!(
        response(&mut output).unwrap_err().code,
        api::ErrorCode::Conflict
    );
    expire(&mut host, &token);
    assert!(host.app.plugins.instances.contains_key(&0));
    host.handle_plugin_event(event);
    assert_eq!(finished(&mut output).state, "succeeded");
    request(&mut host, 0, 6, api::Request::JobCancel { job: token });
    assert_eq!(job(&mut output).state, api::JobState::Succeeded);
    assert_eq!(host.protected_state().plugin_jobs, 0);
}
