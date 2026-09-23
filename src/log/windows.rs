// SPDX-License-Identifier: MPL-2.0

//! Native log ownership and conservative process-liveness checks.
use std::{
    fs::File,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_PARAMETER, ERROR_LOCK_VIOLATION, WAIT_OBJECT_0},
    Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
    System::{
        IO::OVERLAPPED,
        Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
    },
};

pub(super) fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    // A reserved byte outside any possible log content coordinates writers
    // without making Windows' mandatory byte locks block :log-open reads.
    // FAIL_IMMEDIATELY makes the stack-owned OVERLAPPED synchronous even when
    // the supplied handle supports overlapped I/O. Closing File releases it.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.Anonymous.Anonymous.Offset = u32::MAX - 1;
    overlapped.Anonymous.Anonymous.OffsetHigh = u32::MAX;
    if unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    } != 0
    {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Ok(false)
    } else {
        Err(error)
    }
}

pub(super) fn process_is_live(pid: u32) -> bool {
    let handle = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if handle.is_null() {
        // Access-denied and other ambiguous failures must preserve logs.
        return io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER as i32);
    }
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    // A signaled process handle is the only affirmative exit observation.
    (unsafe { WaitForSingleObject(process.as_raw_handle(), 0) }) != WAIT_OBJECT_0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{log::*, private_storage::Directory, test_support::TestRuntimeRoot};
    use std::{
        ffi::OsStr,
        fs,
        io::{Read, Write},
        os::windows::process::CommandExt,
        process::{Child, Command, Stdio},
        time::{Duration, Instant, UNIX_EPOCH},
    };
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    #[test]
    fn native_ownership_allows_reading_and_survives_rotation_until_handle_close() {
        let root = TestRuntimeRoot::new("log-native-ownership").unwrap();
        let path = root.join("explicit.log");
        let (mut first, directory) = open_log_file(&path, true).unwrap();
        first.write_all(b"before rotation\n").unwrap();
        let identity = crate::windows_fs::Identity::of(&first).unwrap();
        let second = directory.append(OsStr::new("explicit.log")).unwrap();
        assert!(!try_lock_exclusive(&second).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"before rotation\n");
        let mut contender = Process(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "log::windows::tests::locked_log_fixture",
                    "--ignored",
                ])
                .env("XDG_CONFIG_HOME", root.join("contender-config"))
                .env("RUNYTE_LOG_PROBE_PATH", &path)
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        contender.expect_success();
        rotate(&mut first, &path, &directory).unwrap();
        assert_eq!(crate::windows_fs::Identity::of(&first).unwrap(), identity);
        assert!(
            !try_lock_exclusive(&second).unwrap(),
            "rotation released ownership"
        );
        first.write_all(b"after rotation\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"after rotation\n");
        assert_eq!(
            fs::read(previous_path(&path)).unwrap(),
            b"before rotation\n"
        );
        drop(first);
        assert!(
            try_lock_exclusive(&second).unwrap(),
            "closing owner did not release lock"
        );
    }

    struct Process(Child);
    impl Process {
        fn expect_success(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = self.0.try_wait().unwrap() {
                    assert!(status.success(), "native log fixture: {status}");
                    return;
                }
                assert!(Instant::now() < deadline, "native log fixture did not exit");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5000) };
        }
    }

    #[test]
    #[ignore = "compiled process fixture invoked by native log tests"]
    fn process_fixture() {
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
    }

    #[test]
    #[ignore = "compiled process fixture invoked by native log ownership test"]
    fn locked_log_fixture() {
        let path = std::path::PathBuf::from(std::env::var_os("RUNYTE_LOG_PROBE_PATH").unwrap());
        let (file, _) = open_log_file(&path, false).unwrap();
        assert!(!try_lock_exclusive(&file).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"before rotation\n");
    }

    #[test]
    fn native_process_probe_distinguishes_live_exited_and_invalid_processes() {
        let root = TestRuntimeRoot::new("log-native-process").unwrap();
        let mut child = Process(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "log::windows::tests::process_fixture",
                    "--ignored",
                ])
                .env("XDG_CONFIG_HOME", root.join("config"))
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        assert!(process_is_live(std::process::id()));
        assert!(process_is_live(child.0.id()));
        assert!(!process_is_live(u32::MAX));
        // The System process is either accessible and live or inaccessible;
        // either way it must be conservatively retained.
        assert!(process_is_live(4));
        child.0.stdin.take().unwrap().write_all(b"q").unwrap();
        child.expect_success();
        // Keep the process handle so this PID cannot be recycled before probe.
        assert!(!process_is_live(child.0.id()));
    }

    #[test]
    fn native_pruning_keeps_live_owned_and_recent_logs_with_their_backups() {
        let root = TestRuntimeRoot::new("log-native-retention").unwrap();
        let storage = Directory::open(root.path(), true).unwrap();
        let stale = [u32::MAX, u32::MAX - 1, u32::MAX - 2];
        for (age, pid) in stale.iter().enumerate() {
            assert!(!process_is_live(*pid));
            let path = default_path(root.path(), Role::Standalone, *pid);
            storage
                .atomic_write(path.file_name().unwrap(), b"stale")
                .unwrap();
            storage
                .atomic_write(previous_path(&path).file_name().unwrap(), b"previous")
                .unwrap();
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_times(
                fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(age as u64 + 1)),
            )
            .unwrap();
        }
        let live = default_path(root.path(), Role::Standalone, std::process::id());
        storage
            .atomic_write(live.file_name().unwrap(), b"live")
            .unwrap();
        storage
            .atomic_write(previous_path(&live).file_name().unwrap(), b"live previous")
            .unwrap();
        storage
            .atomic_write(OsStr::new("notes.log"), b"unrelated")
            .unwrap();
        // The own-PID exclusion is independent of the liveness probe, even
        // when a caller's captured PID no longer names a live process.
        let own_pid = stale[0];
        assert_eq!(prune_standalone_logs(root.path(), own_pid, 1), 2);
        for (index, pid) in stale.iter().enumerate() {
            let path = default_path(root.path(), Role::Standalone, *pid);
            assert_eq!(path.exists(), index != 1);
            assert_eq!(previous_path(&path).exists(), index != 1);
        }
        assert_eq!(fs::read(&live).unwrap(), b"live");
        assert_eq!(fs::read(previous_path(&live)).unwrap(), b"live previous");
        assert!(root.join("notes.log").exists());
    }
}
