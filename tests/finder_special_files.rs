// SPDX-License-Identifier: MPL-2.0

#![cfg(unix)]

use std::{
    ffi::CString,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Self {
        let root = Self(std::env::temp_dir().join(format!(
            "runyte-finder-special-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
        fs::create_dir(&root.0).unwrap();
        root
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

use runyte::file_picker::{FilePreview, ScanScope, scan_files};

fn fifo(path: &Path) {
    let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
}

#[test]
fn finder_special_files_return_without_waiting_for_a_writer() {
    for case in ["ignore", "preview", "snippet"] {
        let root = Root::new(case);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "finder_special_file_child",
                "--ignored",
                "--nocapture",
            ])
            .env("RUNYTE_FINDER_SPECIAL_ROOT", root.path())
            .env("RUNYTE_FINDER_SPECIAL_CASE", case)
            .env("XDG_CONFIG_HOME", root.path().join("configuration"))
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{case} blocked on a special file");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "{case}: {status}");
    }
}

#[test]
#[ignore = "subprocess fixture invoked by the bounded parent test"]
fn finder_special_file_child() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_FINDER_SPECIAL_ROOT").unwrap());
    let case = std::env::var("RUNYTE_FINDER_SPECIAL_CASE").unwrap();
    let pipe = root.join(".ignore");
    fifo(&pipe);
    let alias = root.join(".gitignore");
    symlink(&pipe, &alias).unwrap();
    fs::write(root.join("ordinary.txt"), "ordinary text").unwrap();
    match case.as_str() {
        "ignore" => {
            let (entries, skipped, _) = scan_files(
                &root,
                &ScanScope::ignoring(&root),
                &root.join(".runyte"),
                false,
            )
            .unwrap();
            assert_eq!(skipped, 2);
            assert_eq!(entries.len(), 1);
            // The scanner canonicalizes its root; macOS temporary paths can
            // reach /private/var through the /var symlink.
            assert_eq!(
                entries[0].path,
                root.join("ordinary.txt").canonicalize().unwrap()
            );
        }
        "preview" => {
            for path in [&pipe, &alias, Path::new("/dev/null")] {
                assert!(matches!(
                    FilePreview::from_path(path),
                    FilePreview::Unreadable(_)
                ));
            }
        }
        "snippet" => {
            for path in [&pipe, &alias, Path::new("/dev/null")] {
                assert!(matches!(
                    FilePreview::snippet_from_path(path, 0, vec![]),
                    FilePreview::Unreadable(_)
                ));
            }
        }
        _ => panic!("unknown fixture case"),
    }
    fs::remove_file(&alias).unwrap();
    symlink(root.join("ordinary.txt"), &alias).unwrap();
    assert!(matches!(
        FilePreview::from_path(&alias),
        FilePreview::Text(_)
    ));
}
