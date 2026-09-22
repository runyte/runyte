// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::external_open::{Dispatch, LaunchTicket};
use crate::test_support::TestRuntimeRoot;
use std::sync::mpsc::SyncSender;

fn app(root: &TestRuntimeRoot) -> App {
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(root.path(), ports).unwrap();
    app.programs = ProgramCache::load(Some(root.join("programs")));
    app
}

fn pending_program(app: &mut App, deadline: Instant) -> SyncSender<Result<()>> {
    let (sender, ticket) = LaunchTicket::channel(deadline);
    let ticket = Mutex::new(Some(ticket));
    app.ports.program_opener =
        Box::new(move |_, _| Ok(Dispatch::Pending(ticket.lock().unwrap().take().unwrap())));
    sender
}

#[test]
fn pending_open_keeps_editing_responsive_and_remembers_only_after_acceptance() {
    let root = TestRuntimeRoot::new("open-pending").unwrap();
    let mut app = app(&root);
    let now = Instant::now();
    let complete = pending_program(&mut app, now + Duration::from_secs(5));
    let target = root.join("original image.png");
    app.open_externally(&target, "viewer --fit".into());
    assert!(app.programs.programs().is_empty());
    assert!(!app.poll_external_opens(now));
    press(&mut app, 'i');
    press(&mut app, 'x');
    assert_eq!(app.active_buffer().to_string(), "x");
    app.open_scratch_buffer();
    app.working_directory = root.join("different");
    complete.send(Ok(())).unwrap();
    assert!(app.poll_external_opens(now));
    assert!(app.status.contains(&target.display().to_string()));
    assert_eq!(app.programs.programs(), ["viewer --fit"]);
    assert_eq!(
        ProgramCache::load(Some(root.join("programs"))).programs(),
        ["viewer --fit"]
    );
    assert!(!app.poll_external_opens(now));
}

#[test]
fn failed_and_unknown_launches_do_not_cache_or_retry() {
    let root = TestRuntimeRoot::new("open-errors").unwrap();
    let mut app = app(&root);
    let now = Instant::now();
    let complete = pending_program(&mut app, now + Duration::from_secs(5));
    app.open_externally(&root.join("image.png"), "bad-viewer".into());
    complete
        .send(Err(anyhow::anyhow!("viewer rejected the launch")))
        .unwrap();
    assert!(app.poll_external_opens(now));
    assert!(app.status.contains("viewer rejected"));
    assert!(app.programs.programs().is_empty());

    let late = pending_program(&mut app, now + Duration::from_secs(5));
    app.open_externally(&root.join("image.png"), "late-viewer".into());
    assert!(app.poll_external_opens(now + Duration::from_secs(6)));
    assert!(app.status.contains("unknown"));
    assert!(app.programs.programs().is_empty());
    assert!(
        late.send(Ok(())).is_err(),
        "expired ticket retained a cache update route"
    );
    assert!(!app.poll_external_opens(now + Duration::from_secs(7)));
    assert_eq!(app.unread_notification_counts().errors, 2);
}

#[test]
fn browser_and_directory_completion_keep_the_captured_target_and_skip_program_cache() {
    let root = TestRuntimeRoot::new("open-targets").unwrap();
    let mut app = app(&root);
    let now = Instant::now();
    let (browser_done, browser_ticket) = LaunchTicket::channel(now + Duration::from_secs(5));
    let browser_ticket = Mutex::new(Some(browser_ticket));
    app.ports.browser = Box::new(move |url| {
        assert_eq!(url, "https://example.com/a?b=1&c=2");
        Ok(Dispatch::Pending(
            browser_ticket.lock().unwrap().take().unwrap(),
        ))
    });
    app.open_navigation_target(Some("https://example.com/a?b=1&c=2".into()), None)
        .unwrap();
    let (directory_done, directory_ticket) = LaunchTicket::channel(now + Duration::from_secs(5));
    let directory_ticket = Mutex::new(Some(directory_ticket));
    app.ports.directory_opener = Box::new(move |_| {
        Ok(Dispatch::Pending(
            directory_ticket.lock().unwrap().take().unwrap(),
        ))
    });
    app.open_explorer(Some(root.path().to_owned())).unwrap();
    app.open_explorer_system();
    app.open_scratch_buffer();
    browser_done.send(Ok(())).unwrap();
    assert!(app.poll_external_opens(now));
    assert!(app.status.contains("https://example.com/a?b=1&c=2"));
    directory_done.send(Ok(())).unwrap();
    assert!(app.poll_external_opens(now));
    assert!(app.status.contains(&root.path().display().to_string()));
    assert!(app.status.contains("system file manager"));
    assert!(app.programs.programs().is_empty());
}

#[test]
fn pending_admission_is_bounded_and_editor_drop_retires_completion_tickets() {
    let root = TestRuntimeRoot::new("open-bound").unwrap();
    let mut app = app(&root);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let capture = calls.clone();
    app.ports.program_opener = Box::new(move |_, _| {
        let (sender, ticket) = LaunchTicket::channel(Instant::now() + Duration::from_secs(5));
        capture.lock().unwrap().push(sender);
        Ok(Dispatch::Pending(ticket))
    });
    for _ in 0..17 {
        app.open_externally(&root.join("image.png"), "viewer".into());
    }
    assert_eq!(calls.lock().unwrap().len(), 16);
    assert!(app.status.contains("limit reached"));
    assert!(app.programs.programs().is_empty());
    drop(app);
    for sender in calls.lock().unwrap().drain(..) {
        assert!(sender.send(Ok(())).is_err());
    }
}

#[test]
fn isolated_program_ports_refuse_and_default_acceptance_is_not_remembered() {
    let root = TestRuntimeRoot::new("open-isolated").unwrap();
    let mut app = app(&root);
    app.open_externally(&root.join("image.png"), "viewer".into());
    assert!(app.status.contains("isolated editor"));
    assert!(app.programs.programs().is_empty());
    app.ports.program_opener = Box::new(|program, _| {
        assert!(program.is_empty());
        Ok(Dispatch::Accepted)
    });
    app.open_externally(&root.join("image.png"), String::new());
    assert!(app.status.contains("system default application"));
    assert!(app.programs.programs().is_empty());
}
