// SPDX-License-Identifier: MPL-2.0

//! The skill installer's lock is released when its holder closes it.
//!
//! This is the only test in this binary on purpose. `flock` stays held while
//! any copy of the descriptor is open, and a child forked by a concurrent
//! test holds a copy until it execs; with no other test here, nothing forks
//! between releasing the lock and checking that it is free.

#![cfg(unix)]

use std::{
    fs,
    os::fd::AsRawFd,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("runyte-skills-lock-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::create_dir_all(path.join("project/.runyte")).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .args(args)
            .current_dir(self.0.join("project"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_held_lock_refuses_another_installer_until_its_holder_closes_it() {
    let f = Fixture::new();
    let update = ["skills", "update", "--codex", "--project"];
    let installed = f.run(&["skills", "install", "--codex", "--project"]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.0.join("project/.agents/skills/runyte-mcp/.runyte-skill.lock"))
        .unwrap();
    // SAFETY: the descriptor is live and flock takes no pointers.
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    // Each child exits before the next step, taking its copy of the
    // descriptor with it.
    assert!(!f.run(&update).status.success(), "the lock is held");
    assert!(
        f.run(&["skills", "status", "--codex", "--project"])
            .status
            .success()
    );
    drop(lock);
    let released = f.run(&update);
    assert!(
        released.status.success(),
        "{}",
        String::from_utf8_lossy(&released.stderr)
    );
}
