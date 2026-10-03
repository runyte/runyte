// SPDX-License-Identifier: MPL-2.0

use super::*;
#[cfg(unix)]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "runyte-program-cache-files-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn oversized_program_cache_files_are_discarded_before_parsing() {
    let fixture = Fixture::new();
    for name in [CACHE_FILE, DEFAULT_FILE] {
        fs::write(fixture.0.join(name), "x".repeat(64 * 1024 + 1)).unwrap();
    }
    let cache = ProgramCache::load(Some(fixture.0.clone()));
    assert!(cache.programs().is_empty());
    assert!(cache.default_program().is_none());
}

#[cfg(unix)]
#[test]
fn special_program_cache_files_cannot_block_loading_or_persistence() {
    let fixture = Fixture::new();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "external_open::cache_file_tests::special_cache_fixture",
            "--nocapture",
        ])
        .env("RUNYTE_CACHE_FILE_TEST_ROOT", &fixture.0)
        .env("XDG_CONFIG_HOME", fixture.0.join("config"))
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "cache fixture failed: {status}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("program cache I/O blocked on a special file");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
#[ignore = "owned by the bounded cache fixture"]
fn special_cache_fixture() {
    use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt};
    let root = PathBuf::from(std::env::var_os("RUNYTE_CACHE_FILE_TEST_ROOT").unwrap());
    for name in [CACHE_FILE, DEFAULT_FILE] {
        let path = root.join(name);
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: cpath is NUL-terminated and the fixture owns the parent.
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let mut cache = ProgramCache::load(Some(root.clone()));
        if name == CACHE_FILE {
            assert!(cache.remember("viewer").is_err());
        } else {
            assert!(cache.set_default(Some("viewer")).is_err());
        }
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
        fs::remove_file(&path).unwrap();
    }
    let mut cache = ProgramCache::load(Some(root.clone()));
    cache.set_default(Some("long-viewer-name")).unwrap();
    cache.set_default(Some("v")).unwrap();
    assert_eq!(
        ProgramCache::load(Some(root.clone())).default_program(),
        Some("v")
    );
    let target = root.join("regular-cache");
    fs::write(&target, "old-viewer\n").unwrap();
    fs::remove_file(root.join(CACHE_FILE)).unwrap();
    std::os::unix::fs::symlink(&target, root.join(CACHE_FILE)).unwrap();
    cache.remember("new-viewer").unwrap();
    assert!(
        fs::read_to_string(target)
            .unwrap()
            .starts_with("new-viewer\n")
    );
}
