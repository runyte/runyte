// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn shell_filters_preserve_text_and_parse_only_the_command() {
    let root = crate::test_support::TestRuntimeRoot::new("pipe-process").unwrap();
    let cancel = AtomicBool::new(false);
    for (command, input, expected) in [
        ("sort", "b\na\n", "a\nb\n"),
        ("cat", "é😀\n\n", "é😀\n\n"),
        (
            "cat",
            "$(touch injected); 'quoted'",
            "$(touch injected); 'quoted'",
        ),
        ("tr 'a b' 'A B' | sort", "b\na\n", "A\nB\n"),
        ("cat >/dev/null", "abc", ""),
        ("printf 'x\\n\\n'", "", "x\n\n"),
    ] {
        let outputs = run_inner(
            command,
            root.path(),
            vec![input.into()],
            &cancel,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(outputs, [expected], "{command}");
    }
    assert!(!root.path().join("injected").exists());
}

#[test]
fn failed_exit_invalid_utf8_and_output_limit_are_errors() {
    let root = crate::test_support::TestRuntimeRoot::new("pipe-process").unwrap();
    for (command, reason) in [
        ("printf 'diagnosis' >&2; exit 7", "diagnosis"),
        ("printf '\\377'", "UTF-8"),
        ("yes x", "8 MiB"),
        ("runyte_missing_pipe_program", "not found"),
        ("yes error | head -c 40000 >&2; exit 1", "stderr truncated"),
    ] {
        let error = run_inner(
            command,
            root.path(),
            vec!["input".into()],
            &AtomicBool::new(false),
            Duration::from_secs(5),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(reason), "{command}: {error}");
        assert!(error.len() < STDERR_BYTES + 512);
    }
    let error = invoke(
        Path::new("/nonexistent/runyte-shell"),
        "cat",
        root.path(),
        b"",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(5),
        MAX_BYTES,
    )
    .unwrap_err();
    assert!(error.to_string().contains("No such file"));
}

#[test]
fn sequential_inputs_share_one_output_budget_and_deadline() {
    let root = crate::test_support::TestRuntimeRoot::new("pipe-process").unwrap();
    assert_eq!(
        run_inner(
            "sort",
            root.path(),
            vec!["b\na\n".into(), "d\nc\n".into()],
            &AtomicBool::new(false),
            Duration::from_secs(5)
        )
        .unwrap(),
        ["a\nb\n", "c\nd\n"]
    );
    let error = run_inner(
        "head -c 5000000 /dev/zero",
        root.path(),
        vec!["".into(), "".into()],
        &AtomicBool::new(false),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(error.to_string().contains("8 MiB"));
    let error = run_inner(
        "sleep 10",
        root.path(),
        vec!["".into()],
        &AtomicBool::new(false),
        Duration::from_millis(30),
    )
    .unwrap_err();
    assert!(error.to_string().contains("timed out"));
}

#[test]
fn cancellation_and_success_clean_up_children_without_waiting_for_inherited_pipes() {
    let root = crate::test_support::TestRuntimeRoot::new("pipe-process").unwrap();
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        root.path().join("filter"),
    )
    .unwrap();
    std::fs::write(
        root.path().join("filter.behavior"),
        "sleep 30 &\nprintf '%s' \"$!\" > child.pid\nprintf ready > ready\nwait\n",
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let directory = root.path().to_owned();
    let worker =
        std::thread::spawn(move || run("./filter", &directory, vec!["".into()], worker_cancel));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.path().join("ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    cancel.store(true, Ordering::Release);
    assert!(worker.join().unwrap().0.unwrap_err().contains("cancelled"));
    let start = Instant::now();
    assert_eq!(
        run_inner(
            "sleep 30 & printf done",
            root.path(),
            vec!["".into()],
            &AtomicBool::new(false),
            Duration::from_secs(5)
        )
        .unwrap(),
        ["done"]
    );
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn large_stdin_and_stdout_are_drained_concurrently() {
    let root = crate::test_support::TestRuntimeRoot::new("pipe-process").unwrap();
    let input = "é".repeat(512 * 1024);
    assert_eq!(
        run_inner(
            "cat",
            root.path(),
            vec![input.clone()],
            &AtomicBool::new(false),
            Duration::from_secs(5)
        )
        .unwrap(),
        [input]
    );
}
