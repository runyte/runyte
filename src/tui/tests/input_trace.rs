// SPDX-License-Identifier: MPL-2.0

use std::{
    ffi::CString,
    fs,
    os::unix::{ffi::OsStrExt, fs::FileTypeExt},
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn input_trace_refuses_special_files_without_blocking() {
    let root = runyte::test_support::TestRuntimeRoot::new("input-trace-file").unwrap();
    let pipe = root.path().join("trace");
    let cpath = CString::new(pipe.as_os_str().as_bytes()).unwrap();
    // SAFETY: cpath is NUL-terminated and the fixture owns the directory.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "input_trace_tests::input_trace_fixture",
            "--nocapture",
        ])
        .env("RUNYTE_INPUT_TRACE", &pipe)
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "trace fixture failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("input trace blocked on a FIFO");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "owned by the bounded input trace parent fixture"]
fn input_trace_fixture() {
    let path = PathBuf::from(std::env::var_os("RUNYTE_INPUT_TRACE").unwrap());
    assert!(super::open_input_trace().is_err());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    let target = path.with_extension("fifo");
    fs::rename(&path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(super::open_input_trace().is_err());
    fs::remove_file(&path).unwrap();
    assert!(super::open_input_trace().unwrap().is_some());
    fs::write(&path, "previous trace").unwrap();
    assert!(super::open_input_trace().unwrap().is_some());
    assert!(fs::read(&path).unwrap().is_empty());
}
