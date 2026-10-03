// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{
    ffi::CString,
    os::unix::{ffi::OsStrExt, fs::FileTypeExt},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn special_file_io_refuses_promptly_and_preserves_documents() {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "runyte-special-file-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    )));
    fs::create_dir_all(&fixture.0).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "buffer::special_file_tests::special_file_io_fixture",
            "--nocapture",
        ])
        .env("RUNYTE_SPECIAL_FILE_TEST_ROOT", &fixture.0)
        .env("XDG_CONFIG_HOME", fixture.0.join("config"))
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "special-file fixture failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("document I/O blocked on a special file");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fifo(path: &Path) {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: path is a live NUL-terminated string and the mode is owner-only.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
}

#[test]
#[ignore = "owned by the bounded special-file parent fixture"]
fn special_file_io_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_SPECIAL_FILE_TEST_ROOT").unwrap());
    let pipe = root.join("document.pipe");
    fifo(&pipe);
    let alias = root.join("alias.txt");
    std::os::unix::fs::symlink(&pipe, &alias).unwrap();

    for path in [&pipe, &alias, Path::new("/dev/null")] {
        assert!(!crate::external_open::looks_binary(path));
        assert!(Buffer::open(path).is_err());
        assert!(Buffer::open_bounded(path, 100).is_err());
        assert!(DiskState::inspect(path).is_err());
        assert!(inspect_file_metadata(path).is_none());
        assert!(matches!(
            observe_file(path),
            FileObservation::Unreadable { .. }
        ));
    }

    let mut scratch = Buffer::scratch();
    scratch.apply(&Transaction::insert(0, "unsaved text"));
    assert!(scratch.save_as(pipe.clone(), true).is_err());
    assert!(scratch.path.is_none());
    assert_eq!(scratch.to_string(), "unsaved text");
    assert!(fs::symlink_metadata(&pipe).unwrap().file_type().is_fifo());

    let document = root.join("document.txt");
    fs::write(&document, "original").unwrap();
    let mut buffer = Buffer::open(&document).unwrap();
    buffer.apply(&Transaction::insert(0, "edited "));
    fs::remove_file(&document).unwrap();
    fifo(&document);
    assert!(buffer.reload().is_err());
    assert_eq!(buffer.to_string(), "edited original");
    assert!(buffer.dirty);
    assert!(buffer.save(false).is_err());
    assert!(buffer.save(true).is_err());
    assert!(buffer.undo());
    assert_eq!(buffer.to_string(), "original");
    assert!(
        fs::symlink_metadata(&document)
            .unwrap()
            .file_type()
            .is_fifo()
    );

    // The regular-file contract still follows symlinks and accepts new saves.
    fs::remove_file(&pipe).unwrap();
    fs::write(&pipe, "regular text").unwrap();
    assert_eq!(Buffer::open(&alias).unwrap().to_string(), "regular text");
    assert!(matches!(observe_file(&alias), FileObservation::Text { .. }));
    assert!(inspect_file_metadata(&alias).is_some());
    scratch.save_as(root.join("new.txt"), false).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("new.txt")).unwrap(),
        "unsaved text"
    );

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        // A target replaced after the save's final preflight is swapped back,
        // even when reading the displaced object fails its descriptor check.
        let target = root.join("raced.txt");
        let replacement = root.join("replacement.txt");
        fs::write(&target, "baseline").unwrap();
        let expected = DiskState::inspect(&target).unwrap().unwrap();
        fs::write(&replacement, "replacement").unwrap();
        fs::remove_file(&target).unwrap();
        fifo(&target);
        let error =
            replace_file(&replacement, &target, ReplacePolicy::Expected(&expected)).unwrap_err();
        assert!(error.to_string().contains("restored"));
        assert!(fs::symlink_metadata(&target).unwrap().file_type().is_fifo());
        assert_eq!(fs::read_to_string(&replacement).unwrap(), "replacement");
    }
}
