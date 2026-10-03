// SPDX-License-Identifier: MPL-2.0

//! Opening this process's diagnostic log as an ordinary read-only page.
//!
//! Logging status is process-global: a logger can be installed once, and the
//! degraded status a failed installation records then replaces it for the rest
//! of the process. Every state therefore has to be reached in order, inside a
//! single test, in a binary that owns its own process.

use std::{fs, path::PathBuf, time::Duration};

use runyte::{
    app::App,
    command::parse_colon_command,
    config::Config,
    log::{Level, Logger, Role, Settings, Sink},
};

fn temporary(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "runyte-log-buffer-{}-{nanos}-{name}",
        std::process::id()
    ))
}

/// Runs `:log-open` and reports the status it left behind, whether that status
/// is a failure, and the text of whatever the active pane ends up showing.
fn open_log(app: &mut App) -> (String, bool, String) {
    app.execute(parse_colon_command("log-open").unwrap())
        .unwrap();
    (
        app.status.clone(),
        app.status_error,
        app.active_buffer().to_string(),
    )
}

#[test]
fn the_log_page_reports_what_this_process_can_actually_read() {
    let root = temporary("root");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();

    let (message, error, text) = open_log(&mut app);
    assert!(error, "{message}");
    assert_eq!(message, "no diagnostic log is installed for this process");
    let empty = text;

    let path = root.join("standalone.log");
    let logger = Logger::start(
        Settings::new(Level::Info, Role::Standalone),
        Sink::file(&path),
    )
    .unwrap();
    runyte::log::install(logger);
    runyte::log::emit(Level::Info, "test", "a record this page must show");

    let (_, error, text) = open_log(&mut app);
    assert!(!error, "opening an installed log is not a failure");
    assert_ne!(text, empty, "the page replaced the buffer that was showing");
    let header = text.lines().next().unwrap();
    assert!(header.contains(&path.display().to_string()), "{header}");
    assert!(header.contains("standalone owner"), "{header}");
    assert!(
        header.contains("info"),
        "the header names the level being recorded: {header}"
    );
    assert!(
        text.contains("a record this page must show"),
        "the queue is drained before the file is read: {text}"
    );

    // A degraded status replaces the installed one, which is how a logger that
    // never reached a destination is reported.
    runyte::log::note_unavailable(
        Role::Standalone,
        None,
        "cannot open the diagnostic log".to_owned(),
    );
    let (message, error, _) = open_log(&mut app);
    assert!(error, "{message}");
    assert_eq!(
        message,
        "this process's diagnostic log has no file destination"
    );

    let missing = root.join("gone.log");
    runyte::log::note_unavailable(
        Role::Standalone,
        Some(missing.clone()),
        "cannot open the diagnostic log".to_owned(),
    );
    let (message, error, _) = open_log(&mut app);
    assert!(error, "{message}");
    assert!(
        message.contains(&missing.display().to_string()),
        "a destination that cannot be read is named: {message}"
    );

    runyte::log::flush(Duration::from_secs(1));
    runyte::log::shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn log_open_refuses_a_replaced_fifo_without_waiting_for_a_writer() {
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let root = temporary("special-file");
    fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "special_log_file_fixture",
            "--nocapture",
        ])
        .env("RUNYTE_LOG_FILE_FIXTURE_ROOT", &root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    fs::remove_dir_all(&root).unwrap();
    assert!(
        status.is_some_and(|status| status.success()),
        "log-open blocked or the fixture failed: {status:?}"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "owned by the bounded log-file parent fixture"]
fn special_log_file_fixture() {
    use runyte::{app::CommandOutcome, headless::HeadlessEditor};
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let root = PathBuf::from(std::env::var_os("RUNYTE_LOG_FILE_FIXTURE_ROOT").unwrap());
    let path = root.join("standalone.log");
    runyte::log::install(
        Logger::start(
            Settings::new(Level::Info, Role::Standalone),
            Sink::file(&path),
        )
        .unwrap(),
    );
    runyte::log::flush(Duration::from_secs(1));
    fs::remove_file(&path).unwrap();
    let encoded = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: encoded is a live NUL-terminated pathname; the mode is owner-only.
    assert_eq!(unsafe { libc::mkfifo(encoded.as_ptr(), 0o600) }, 0);

    let mut editor = HeadlessEditor::with_text_in(&root, "unsaved document").unwrap();
    let outcome = editor
        .execute(parse_colon_command("log-open").unwrap())
        .unwrap();
    assert!(
        matches!(outcome, CommandOutcome::UserError(ref message) if message.contains("regular file")),
        "{outcome:?}"
    );
    assert_eq!(editor.active_text(), "unsaved document");

    fs::remove_file(&path).unwrap();
    let target = root.join("regular.log");
    fs::write(&target, "retained regular log\n").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let outcome = editor
        .execute(parse_colon_command("log-open").unwrap())
        .unwrap();
    assert!(
        !matches!(outcome, CommandOutcome::UserError(_)),
        "{outcome:?}"
    );
    assert!(editor.active_text().contains("retained regular log"));
    runyte::log::shutdown();
}
