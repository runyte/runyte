// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::{
    fs,
    os::unix::fs::symlink,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "runyte-system-open-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn helper(&self, behavior: &str) -> PathBuf {
        let executable = self.0.join("handler with spaces");
        symlink(
            concat!(env!("CARGO_MANIFEST_DIR"), "/src/fixtures/stand-in"),
            &executable,
        )
        .unwrap();
        fs::write(self.0.join("handler with spaces.behavior"), behavior).unwrap();
        executable
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "opener fixture did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn accepts_only_complete_http_urls_without_credentials_or_control_characters() {
    for valid in [
        "https://example.test/path?q=one%20two#fragment",
        "http://localhost:8080/",
        "https://[::1]/",
        "HTTPS://example.test/a?query=$(literal)",
    ] {
        let prepared = prepare(Path::new("/absent-workspace"), Target::Url(valid.into())).unwrap();
        assert_eq!(prepared.argument, valid);
        assert!(!format!("{prepared:?}").contains(valid));
    }
    for invalid in [
        "",
        "https://",
        "https:///example.test",
        "https://:443/",
        "http://[oops]/",
        "http://example.test:99999/",
        "https://user:secret@example.test",
        "https://@example.test",
        " https://example.test",
        "https://example.test/a b",
        "https://example.test/a\u{2028}b",
        "https://example.test\n",
        "https://example.test/\u{85}",
        "https:\\example.test",
        "https:example.test",
        "file:///tmp/file",
        "data:text/plain,hello",
        "javascript:alert(1)",
        "mailto:user@example.test",
        "custom://example.test",
    ] {
        assert_eq!(
            prepare(Path::new("/"), Target::Url(invalid.into())).unwrap_err(),
            Error::InvalidTarget,
            "{invalid:?}"
        );
    }
    assert_eq!(
        prepare(
            Path::new("/"),
            Target::Url(format!(
                "https://example.test/{}",
                "x".repeat(MAX_TARGET_BYTES)
            ))
        )
        .unwrap_err(),
        Error::InvalidTarget
    );
}

#[test]
fn files_are_canonical_existing_regular_targets_inside_the_workspace() {
    let root = Root::new();
    let outside = Root::new();
    fs::write(root.0.join("a file [x].bin"), [0, 255]).unwrap();
    fs::write(outside.0.join("outside"), []).unwrap();
    symlink(root.0.join("a file [x].bin"), root.0.join("inside-link")).unwrap();
    symlink(outside.0.join("outside"), root.0.join("outside-link")).unwrap();
    let prepared = prepare(&root.0, Target::File("inside-link".into())).unwrap();
    assert_eq!(prepared.argument, root.0.join("a file [x].bin").as_os_str());
    for path in ["missing", ".", "outside-link", "bad\nname"] {
        assert_eq!(
            prepare(&root.0, Target::File(path.into())).unwrap_err(),
            Error::InvalidTarget
        );
    }
    assert!(
        prepare(
            &root.0,
            Target::File(outside.0.join("outside").to_string_lossy().into())
        )
        .is_err()
    );
    let fifo = root.0.join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: a valid temporary path and mode, creating no executable or process.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(prepare(&root.0, Target::File("fifo".into())).is_err());
}

#[test]
fn fixed_platform_handlers_do_not_accept_plugin_selected_programs() {
    assert_eq!(
        platform_handler(super::super::OpenPlatform::Linux),
        Some("xdg-open")
    );
    assert_eq!(
        platform_handler(super::super::OpenPlatform::MacOs),
        Some("/usr/bin/open")
    );
    assert_eq!(
        platform_handler(super::super::OpenPlatform::Unsupported),
        None
    );
}

#[test]
fn exact_single_argument_and_independent_group_are_preserved_through_reap() {
    let root = Root::new();
    let helper = root.helper(
        "printf '%s\\n%s\\n%s\\n' \"$#\" \"$1\" \"$$\" > \"$0.args\"\nread ignored < \"$0.gate\"\n",
    );
    let gate = root.0.join("handler with spaces.gate");
    let name = std::ffi::CString::new(gate.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: a valid path within this test's temporary directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let active = Arc::new(AtomicUsize::new(0));
    let target = "https://example.test/a?x=$(literal)&space=one%20two";
    launch_with(
        prepare(&root.0, Target::Url(target.into())).unwrap(),
        helper.as_os_str(),
        active.clone(),
    )
    .unwrap();
    let record = root.0.join("handler with spaces.args");
    until(|| fs::read_to_string(&record).is_ok_and(|text| text.lines().count() == 3));
    let record = fs::read_to_string(record).unwrap();
    let mut lines = record.lines();
    assert_eq!(lines.next(), Some("1"));
    assert_eq!(lines.next(), Some(target));
    let pid: i32 = lines.next().unwrap().parse().unwrap();
    // SAFETY: the helper is alive, blocked on our unreleased FIFO gate.
    assert_eq!(unsafe { libc::getpgid(pid) }, pid);
    assert_eq!(active.load(Ordering::Acquire), 1);
    // The launch call has returned and its Prepared is gone; the independent
    // handler remains alive until its own work ends.
    fs::write(gate, b"release\n").unwrap();
    until(|| active.load(Ordering::Acquire) == 0);
    // SAFETY: signal zero only checks whether the already-reaped child remains.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}

#[test]
fn admission_and_failed_spawn_release_only_their_own_bounded_record() {
    let root = Root::new();
    let active = Arc::new(AtomicUsize::new(MAX_OPENERS));
    let target = || prepare(&root.0, Target::Url("https://example.test".into())).unwrap();
    assert_eq!(
        launch_with(target(), OsStr::new("unused"), active.clone()).unwrap_err(),
        Error::Busy
    );
    assert_eq!(active.load(Ordering::Acquire), MAX_OPENERS);
    active.store(0, Ordering::Release);
    assert_eq!(
        launch_with(
            target(),
            root.0.join("absent-handler").as_os_str(),
            active.clone()
        )
        .unwrap_err(),
        Error::Unavailable
    );
    until(|| active.load(Ordering::Acquire) == 0);
}

#[test]
fn accepted_spawn_does_not_claim_the_handler_completed_its_action() {
    let root = Root::new();
    let helper = root.helper("exit 17\n");
    let active = Arc::new(AtomicUsize::new(0));
    let prepared = prepare(&root.0, Target::Url("https://example.test".into())).unwrap();
    assert_eq!(
        launch_with(prepared, helper.as_os_str(), active.clone()),
        Ok(())
    );
    until(|| active.load(Ordering::Acquire) == 0);
}

#[test]
fn reaper_thread_admission_failure_never_spawns_and_returns_the_record() {
    let root = Root::new();
    let helper = root.helper("printf started > \"$0.started\"\n");
    let active = Arc::new(AtomicUsize::new(0));
    let prepared = prepare(&root.0, Target::Url("https://example.test".into())).unwrap();
    let result = launch_using(prepared, helper.as_os_str(), active.clone(), |work| {
        drop(work);
        Err(std::io::Error::other("injected thread admission failure"))
    });
    assert_eq!(result, Err(Error::Unavailable));
    assert_eq!(active.load(Ordering::Acquire), 0);
    assert!(!root.0.join("handler with spaces.started").exists());
}

#[test]
fn startup_timeout_keeps_late_opener_owned_and_never_retries_or_kills_it() {
    let root = Root::new();
    let helper = root.helper("printf started > \"$0.started\"\n");
    let active = Arc::new(AtomicUsize::new(0));
    let prepared = prepare(&root.0, Target::Url("https://example.test".into())).unwrap();
    let (release, gate) = mpsc::channel();
    let result = launch_using(prepared, helper.as_os_str(), active.clone(), |work| {
        std::thread::Builder::new()
            .name("test-opener-gate".into())
            .spawn(move || {
                let _ = gate.recv();
                work();
            })
            .map(|_| ())
    });
    assert_eq!(result, Err(Error::OutcomeUnknown));
    assert_eq!(active.load(Ordering::Acquire), 1);
    assert!(!root.0.join("handler with spaces.started").exists());
    release.send(()).unwrap();
    until(|| active.load(Ordering::Acquire) == 0);
    assert_eq!(
        fs::read(root.0.join("handler with spaces.started")).unwrap(),
        b"started"
    );
}
