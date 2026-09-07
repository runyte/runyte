// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::time::Duration;

fn channels() -> (SyntaxHandle, SyntaxEvents, Arc<Shared>) {
    let shared = Arc::new(Shared::default());
    (
        SyntaxHandle(Arc::new(Sender(Arc::clone(&shared)))),
        SyntaxEvents {
            shared: Arc::clone(&shared),
        },
        shared,
    )
}

fn request(registry: &Registry, buffer: usize, generation: u64, text: &str) -> ParseRequest {
    ParseRequest::full(
        buffer,
        generation,
        registry.language_for_name("rust").unwrap(),
        Text::from_str(text),
    )
}

#[tokio::test]
async fn initial_parse_checkpoint_catches_up_without_publishing_old_text() {
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    let first = request(&registry, 0, 1, "fn first() {}\n");
    let latest = request(&registry, 0, 1, "// newest\nfn first() {}\n");
    let revision = latest.target.revision();
    worker.send(first);
    let sender = worker.clone();
    let parser = std::thread::spawn(move || {
        let calls = std::cell::Cell::new(0);
        run_worker_with_parse(registry, shared, |request, checkpoint, registry| {
            let call = calls.get();
            calls.set(call + 1);
            if call == 0 {
                assert!(checkpoint.is_none());
                sender.send(latest.clone());
            } else {
                assert!(checkpoint.is_some(), "reuse the initial parsed snapshot");
            }
            parse(request, checkpoint, registry)
        });
        assert_eq!(calls.get(), 2);
    });
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.text_revision, revision);
    assert!(event.syntax.is_some());
    drop(events);
    parser.join().unwrap();
}

#[tokio::test]
async fn cancellation_retires_active_work_and_other_buffers_still_complete() {
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    worker.send(request(&registry, 0, 1, "fn cancelled() {}"));
    worker.send(request(&registry, 1, 2, "fn kept() {}"));
    let sender = worker.clone();
    let parser = std::thread::spawn(move || {
        run_worker_with_parse(registry, shared, |request, checkpoint, registry| {
            if request.buffer == 0 {
                sender.cancel(0);
            }
            parse(request, checkpoint, registry)
        })
    });
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.buffer, 1);
    assert!(worker.0.0.lock().completed.is_empty());
    drop(events);
    parser.join().unwrap();
}

#[tokio::test]
async fn parser_panic_settles_one_request_and_preserves_other_work() {
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    worker.send(request(&registry, 0, 1, "fn broken() {}"));
    worker.send(request(&registry, 1, 2, "fn kept() {}"));
    let parser = std::thread::spawn(move || {
        run_worker_with_parse(registry, shared, |request, checkpoint, registry| {
            assert_ne!(request.buffer, 0, "injected parser failure");
            parse(request, checkpoint, registry)
        })
    });
    let failed = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.buffer, 0);
    assert_eq!(failed.failure.as_deref(), Some("syntax parser panicked"));
    let kept = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.buffer, 1);
    assert!(kept.syntax.is_some());
    drop(events);
    parser.join().unwrap();
}

#[test]
fn shutdown_does_not_join_an_active_parser() {
    let registry = Arc::new(Registry::new());
    let (worker, events, shared) = channels();
    worker.send(request(&registry, 0, 1, "fn held() {}"));
    let (started, wait_started) = std::sync::mpsc::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let parser = std::thread::spawn(move || {
        run_worker_with_parse(registry, shared, |_, _, _| {
            started.send(()).unwrap();
            wait_release.recv_timeout(Duration::from_secs(5)).unwrap();
            None
        })
    });
    wait_started.recv_timeout(Duration::from_secs(5)).unwrap();
    let (stopped, wait_stopped) = std::sync::mpsc::channel();
    let stopping = std::thread::spawn(move || {
        drop(worker);
        drop(events);
        stopped.send(()).unwrap();
    });
    let result = wait_stopped.recv_timeout(Duration::from_secs(2));
    release.send(()).unwrap();
    stopping.join().unwrap();
    parser.join().unwrap();
    assert!(
        result.is_ok(),
        "shutdown must finish before the parser is released"
    );
}

#[tokio::test]
async fn request_and_completion_storage_coalesce_per_buffer() {
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    for generation in 0..100 {
        worker.send(request(&registry, 0, generation, "fn latest() {}"));
    }
    assert_eq!(shared.lock().pending.len(), 1);
    assert_eq!(shared.lock().order.len(), 1);
    worker.cancel(0);
    assert!(shared.lock().pending.is_empty());
    assert!(shared.lock().order.is_empty());
    assert!(shared.lock().generations.is_empty());
    drop(worker);
    assert!(events.recv().await.is_none());
}

#[tokio::test]
async fn active_buffer_priority_alternates_with_fifo_work() {
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    for buffer in [1, 2, 0] {
        worker.send(request(&registry, buffer, 1, "fn initial() {}"));
    }
    worker.prioritize(0);
    let history = Arc::new(Mutex::new(Vec::new()));
    let parsed = Arc::clone(&history);
    let sender = worker.clone();
    let parser = std::thread::spawn(move || {
        run_worker_with_parse(registry, shared, |current, checkpoint, registry| {
            let mut history = parsed.lock().unwrap();
            history.push(current.buffer);
            if history.len() == 1 {
                sender.send(request(registry, 0, 1, "// latest\nfn initial() {}"));
            }
            drop(history);
            parse(current, checkpoint, registry)
        })
    });
    for _ in 0..3 {
        assert!(
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
                .syntax
                .is_some()
        );
    }
    drop(events);
    parser.join().unwrap();
    assert_eq!(*history.lock().unwrap(), [0, 1, 0, 2]);
}

#[tokio::test]
async fn an_unread_completion_is_replaced_without_losing_another_buffer() {
    async fn completed(shared: &Shared, buffer: usize, generation: u64) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let ready = shared.finished.notified();
                if shared
                    .lock()
                    .completed
                    .get(&buffer)
                    .is_some_and(|(event, _)| event.generation == generation)
                {
                    return;
                }
                ready.await;
            }
        })
        .await
        .unwrap();
    }
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    let observed = Arc::clone(&shared);
    let parser_registry = Arc::clone(&registry);
    let parser = std::thread::spawn(move || run_worker(parser_registry, shared));
    worker.send(request(&registry, 1, 1, "fn retained() {}"));
    completed(&observed, 1, 1).await;
    worker.send(request(&registry, 0, 1, "fn old() {}"));
    completed(&observed, 0, 1).await;
    worker.send(request(&registry, 0, 2, "fn new() {}"));
    completed(&observed, 0, 2).await;
    assert_eq!(observed.lock().completed.len(), 2);
    assert_eq!(observed.lock().completion_order.len(), 2);
    let retained = events.recv().await.unwrap();
    let newest = events.recv().await.unwrap();
    assert_eq!((retained.buffer, retained.generation), (1, 1));
    assert_eq!((newest.buffer, newest.generation), (0, 2));
    drop(events);
    parser.join().unwrap();
}

#[tokio::test]
async fn retired_initial_trees_are_disposed_on_the_worker_without_holding_the_queue() {
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    let observed = Arc::clone(&shared);
    let input_thread = std::thread::current().id();
    let (disposing, wait_disposing) = std::sync::mpsc::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let parser_registry = Arc::clone(&registry);
    let parser = std::thread::spawn(move || {
        let first = std::cell::Cell::new(true);
        run_worker_with_disposal(parser_registry, Arc::clone(&shared), parse, || {
            assert_ne!(std::thread::current().id(), input_thread);
            assert!(
                shared.queue.try_lock().is_ok(),
                "no tree disposal may hold the queue mutex"
            );
            if first.replace(false) {
                disposing.send(()).unwrap();
                wait_release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        });
    });
    worker.send(request(&registry, 0, 1, "fn obsolete() {}"));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ready = observed.finished.notified();
            if observed.lock().completed.contains_key(&0) {
                break;
            }
            ready.await;
        }
    })
    .await
    .unwrap();
    // The only initial tree is still in the unread completion. Cancellation
    // must return before the worker is permitted to destroy that tree.
    worker.cancel(0);
    wait_disposing.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(worker.send(request(&registry, 1, 2, "fn input_continues() {}")));
    worker.cancel(1);
    assert!(worker.send(request(&registry, 1, 3, "fn latest() {}")));
    release.send(()).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((event.buffer, event.generation), (1, 3));
    drop(events);
    parser.join().unwrap();
    assert!(observed.lock().pending.is_empty());
    assert!(observed.lock().completed.is_empty());
}

#[tokio::test]
async fn superseded_unread_initial_completion_is_reused_as_a_checkpoint() {
    let registry = Arc::new(Registry::new());
    let (worker, mut events, shared) = channels();
    let observed = Arc::clone(&shared);
    let parser_registry = Arc::clone(&registry);
    let parser = std::thread::spawn(move || {
        let calls = std::cell::Cell::new(0);
        run_worker_with_parse(parser_registry, shared, |request, checkpoint, registry| {
            assert_eq!(checkpoint.is_some(), calls.get() == 1);
            calls.set(calls.get() + 1);
            parse(request, checkpoint, registry)
        });
        assert_eq!(calls.get(), 2);
    });
    worker.send(request(&registry, 0, 1, "fn initial() {}"));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ready = observed.finished.notified();
            if observed.lock().completed.contains_key(&0) {
                break;
            }
            ready.await;
        }
    })
    .await
    .unwrap();
    worker.send(request(&registry, 0, 1, "// changed\nfn initial() {}"));
    let result = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.syntax.unwrap().revision().get(), 1);
    drop(events);
    parser.join().unwrap();
}
