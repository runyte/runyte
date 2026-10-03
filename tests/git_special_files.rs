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
            "runyte-git-special-{label}-{}-{}",
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

use runyte::git::{
    GitCliProvider, GitProvider, NetworkRequest, NetworkScope, Repository, read_workspace_git_facts,
};

fn fifo(path: &Path) {
    let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
}

#[test]
fn git_metadata_special_files_return_without_waiting_for_a_writer() {
    for child_test in [
        "workspace_facts_special_file_child",
        "network_shallow_special_file_child",
    ] {
        let root = Root::new(child_test);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", child_test, "--ignored", "--nocapture"])
            .env("RUNYTE_GIT_SPECIAL_FILE_ROOT", root.path())
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
                panic!("{child_test} blocked on a special metadata file");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "{child_test}: {status}");
    }
}

#[test]
#[ignore = "subprocess fixture invoked by the bounded parent test"]
fn workspace_facts_special_file_child() {
    let root = std::path::PathBuf::from(std::env::var_os("RUNYTE_GIT_SPECIAL_FILE_ROOT").unwrap());
    for name in [".git", "HEAD", "commondir", "config"] {
        let project = root.join(name);
        fs::create_dir(&project).unwrap();
        if name == ".git" {
            fifo(&project.join(".git"));
            assert!(read_workspace_git_facts(&project).is_none());
        } else {
            fs::create_dir(project.join(".git")).unwrap();
            fifo(&project.join(".git").join(name));
            let facts = read_workspace_git_facts(&project).unwrap();
            assert!(facts.branch.is_none());
            assert!(facts.remote.is_none());
        }
    }
}

#[test]
#[ignore = "subprocess fixture invoked by the bounded parent test"]
fn network_shallow_special_file_child() {
    let root = std::path::PathBuf::from(std::env::var_os("RUNYTE_GIT_SPECIAL_FILE_ROOT").unwrap());
    let program = root.join("git-fixture");
    symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    fs::write(
        program.with_extension("behavior"),
        "case \"$3\" in\n--verify) exit 1 ;;\n--git-path) printf 'shallow\\n' ;;\n*) exit 0 ;;\nesac\n",
    )
    .unwrap();
    fifo(&root.join("shallow"));
    let error = GitCliProvider::new(program)
        .network_page(
            &Repository::new(&root),
            &NetworkRequest {
                scope: NetworkScope::Head,
                cursor: None,
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("shallow boundary"));
}
