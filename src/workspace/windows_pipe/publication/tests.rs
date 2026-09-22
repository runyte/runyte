// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    test_support::TestRuntimeRoot,
    workspace::windows_endpoint::{EndpointLocation, RegistrySet},
};
use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::Poll,
    time::Duration,
};
use tokio::time::{Instant, timeout_at};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn fixture() -> (TestRuntimeRoot, EndpointLocation, NameStore, Publication) {
    let root = TestRuntimeRoot::new("publication-worker").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let location = EndpointLocation::new(
        &project,
        root.join("endpoint"),
        RegistrySet::open(&[root.join("registry")]).unwrap(),
    )
    .unwrap();
    let names = NameStore::open(&root.join("state")).unwrap();
    let publication = location
        .prepare_named(&names, Some("initial".into()))
        .unwrap()
        .publish()
        .unwrap();
    (root, location, names, publication)
}
async fn pending<F: Future>(future: &mut Pin<Box<F>>) {
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}
async fn named(snapshot: &mut watch::Receiver<EndpointMetadata>, expected: &str) {
    timeout_at(deadline(), async {
        loop {
            if snapshot.borrow_and_update().name.as_deref() == Some(expected) {
                return;
            }
            snapshot.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
fn blocked(
    publication: Publication,
    names: NameStore,
) -> (
    Worker,
    oneshot::Receiver<()>,
    mpsc::Sender<()>,
    Arc<AtomicUsize>,
) {
    let (entered, entry) = oneshot::channel();
    let (release, released) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut entered = Some(entered);
    let worker = Worker::start_with(publication, names, move |publication, names, name| {
        if let Some(entered) = entered.take() {
            let _ = entered.send(());
            released
                .recv_timeout(Duration::from_secs(5))
                .map_err(io::Error::other)?;
        }
        observed.fetch_add(1, Ordering::SeqCst);
        publication.rename(names, name)
    })
    .unwrap();
    (worker, entry, release, calls)
}

#[test]
fn canceled_queued_operation_is_skipped_and_queue_capacity_is_one() {
    runtime().block_on(async {
        let (_root, location, names, publication) = fixture();
        let (mut worker, entered, release, calls) = blocked(publication, names.clone());
        let requests = worker.requests();
        let first = requests.rename("first").unwrap();
        timeout_at(deadline(), entered).await.unwrap().unwrap();
        let skipped = requests.rename("skipped").unwrap();
        assert_eq!(
            requests.rename("overflow").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(skipped);
        release.send(()).unwrap();
        assert_eq!(first.wait().await.unwrap().name.as_deref(), Some("first"));
        let deadline = deadline();
        let last = loop {
            match requests.rename("last") {
                Ok(ticket) => break ticket,
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                    assert!(Instant::now() < deadline);
                    tokio::task::yield_now().await;
                }
            }
        };
        assert_eq!(last.wait().await.unwrap().name.as_deref(), Some("last"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            names.load(&worker.initial.id).unwrap().as_deref(),
            Some("last")
        );
        assert!(location.read_ready().unwrap().is_some());
        worker.retire().await.unwrap();
        assert!(worker.thread.is_none());
        assert!(location.read_ready().unwrap().is_none());
    });
}

#[test]
fn canceled_started_request_still_commits_and_updates_current_snapshot() {
    runtime().block_on(async {
        let (_root, location, names, publication) = fixture();
        let (mut worker, entered, release, _) = blocked(publication, names.clone());
        let mut snapshot = worker.snapshot();
        let ticket = worker.requests().rename("committed").unwrap();
        timeout_at(deadline(), entered).await.unwrap().unwrap();
        let mut waiting = Box::pin(ticket.wait());
        pending(&mut waiting).await;
        drop(waiting);
        release.send(()).unwrap();
        named(&mut snapshot, "committed").await;
        assert_eq!(
            location.read_ready().unwrap().unwrap().name.as_deref(),
            Some("committed")
        );
        assert_eq!(
            names.load(&worker.initial.id).unwrap().as_deref(),
            Some("committed")
        );
        assert_eq!(worker.initial().name.as_deref(), Some("initial"));
        worker.retire().await.unwrap();
    });
}

#[test]
fn canceled_retirement_resumes_and_rejects_queued_work_without_losing_active_owner() {
    runtime().block_on(async {
        let (_root, location, names, publication) = fixture();
        let (mut worker, entered, release, calls) = blocked(publication, names);
        let requests = worker.requests();
        let first = requests.rename("first").unwrap();
        timeout_at(deadline(), entered).await.unwrap().unwrap();
        let queued = requests.rename("never-started").unwrap();
        let mut retirement = Box::pin(worker.retire());
        pending(&mut retirement).await;
        drop(retirement);
        assert!(location.read_ready().unwrap().is_some());
        assert_eq!(
            requests.rename("late").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        release.send(()).unwrap();
        first.wait().await.unwrap();
        assert!(queued.wait().await.is_err());
        worker.retire().await.unwrap();
        worker.retire().await.unwrap();
        assert!(worker.thread.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(location.read_ready().unwrap().is_none());
    });
}

#[test]
fn worker_panic_preserves_committed_metadata_and_publication_until_ordered_retirement() {
    runtime().block_on(async {
        let (_root, location, names, publication) = fixture();
        let mut worker = Worker::start_with(publication, names, |publication, names, name| {
            publication.rename(names, name)?;
            panic!("injected post-commit worker panic");
        })
        .unwrap();
        let mut failed = worker.failure();
        let ticket = worker.requests().rename("committed-before-panic").unwrap();
        assert!(ticket.wait().await.is_err());
        timeout_at(deadline(), async {
            while !*failed.borrow_and_update() {
                failed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            worker.snapshot().borrow().name.as_deref(),
            Some("committed-before-panic")
        );
        assert!(
            location.read_ready().unwrap().is_some(),
            "worker failure must retain publication for its transport owner"
        );
        assert_eq!(
            worker.requests().rename("refused").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(worker.retire().await.is_err());
        assert!(worker.thread.is_none());
        assert!(location.read_ready().unwrap().is_none());
    });
}

#[test]
fn dropping_worker_waits_for_started_work_then_retires_and_joins() {
    runtime().block_on(async {
        let (_root, location, names, publication) = fixture();
        let (worker, entered, release, _) = blocked(publication, names);
        let requests = worker.requests();
        let ticket = requests.rename("finishing").unwrap();
        timeout_at(deadline(), entered).await.unwrap().unwrap();
        let shared = worker.shared.clone();
        struct JoinedDrop {
            release: Option<mpsc::Sender<()>>,
            thread: Option<thread::JoinHandle<()>>,
        }
        impl Drop for JoinedDrop {
            fn drop(&mut self) {
                if let Some(release) = self.release.take() {
                    let _ = release.send(());
                }
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
            }
        }
        let mut dropper = JoinedDrop {
            release: Some(release),
            thread: Some(thread::spawn(move || drop(worker))),
        };
        timeout_at(deadline(), async {
            while !shared.lock().retiring {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(location.read_ready().unwrap().is_some());
        dropper.release.take().unwrap().send(()).unwrap();
        ticket.wait().await.unwrap();
        dropper.thread.take().unwrap().join().unwrap();
        assert!(location.read_ready().unwrap().is_none());
    });
}

#[test]
fn invalid_names_are_refused_before_queue_ownership_and_store_is_fixed() {
    runtime().block_on(async {
        let (_root, location, names, publication) = fixture();
        let mut worker = Worker::start(publication, names.clone()).unwrap();
        for name in ["", " padded ", "newline\n", &"x".repeat(65)] {
            assert_eq!(
                worker.requests().rename(name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        worker
            .requests()
            .rename("selected-store")
            .unwrap()
            .wait()
            .await
            .unwrap();
        assert_eq!(
            names.load(&worker.initial.id).unwrap().as_deref(),
            Some("selected-store")
        );
        worker.retire().await.unwrap();
        assert!(location.read_ready().unwrap().is_none());
    });
}

#[test]
fn canceled_failed_update_retains_recovery_ledger_until_explicit_retirement() {
    runtime().block_on(async {
        let (_root, location, names, publication) = fixture();
        let (entered, entry) = oneshot::channel();
        let (release, released) = mpsc::channel();
        let mut entered = Some(entered);
        let mut worker = Worker::start_with(
            publication,
            names.clone(),
            move |publication, names, name| {
                let result = publication.fixture_pending_name_update(names, name);
                assert!(publication.rename_recovery_pending());
                if let Some(entered) = entered.take() {
                    let _ = entered.send(());
                }
                released
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(io::Error::other)?;
                result
            },
        )
        .unwrap();
        let ticket = worker.requests().rename("partially-installed").unwrap();
        timeout_at(deadline(), entry).await.unwrap().unwrap();
        assert!(location.read_ready().unwrap().is_some());
        drop(ticket);
        release.send(()).unwrap();
        worker.retire().await.unwrap();
        assert!(worker.thread.is_none());
        assert!(location.read_ready().unwrap().is_none());
        assert!(
            location
                .observe_registrations()
                .unwrap()
                .candidates
                .is_empty()
        );
        assert!(
            names.load(&worker.initial.id).unwrap().is_none(),
            "the original absent stored name was restored"
        );
        drop(location.prepare(None).unwrap());
    });
}
