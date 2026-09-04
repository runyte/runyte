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
