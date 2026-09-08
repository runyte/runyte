// SPDX-License-Identifier: MPL-2.0

//! Observe a direct child without reaping the process-group identity it anchors.
use std::{
    io,
    process::{Child, ExitStatus},
};

#[cfg(target_os = "macos")]
pub(crate) struct ChildExitObserver {
    pid: libc::pid_t,
    already_completed: Option<DarwinChildCompletion>,
}

#[cfg(not(target_os = "macos"))]
pub(crate) struct ChildExitObserver;

#[cfg(target_os = "macos")]
impl ChildExitObserver {
    pub(crate) fn new(child: &std::process::Child) -> io::Result<Self> {
        let pid = i32::try_from(child.id()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "child PID does not fit pid_t")
        })?;
        // A direct, unreaped Child remains in either XNU's live or zombie
        // process table. Query both tables so the observer is level-triggered
        // and has no spawn-to-registration gap. INEXIT precedes SZOMB and may
        // not authorize group cleanup: signalling the group then would also
        // signal the still-exiting process leader and could replace its successful
        // status with SIGKILL.
        let already_completed = darwin_process_completion(pid)?;
        Ok(Self {
            pid,
            already_completed,
        })
    }

    pub(crate) fn completion(&self) -> io::Result<Option<DarwinChildCompletion>> {
        if self.already_completed.is_some() {
            return Ok(self.already_completed);
        }
        darwin_process_completion(self.pid)
    }
}

#[cfg(target_os = "macos")]
const DARWIN_PROC_FLAG_INEXIT: u32 = 4;

#[cfg(target_os = "macos")]
const DARWIN_INCLUDE_ZOMBIES: u64 = 1;

#[cfg(target_os = "macos")]
fn darwin_process_completion(pid: libc::pid_t) -> io::Result<Option<DarwinChildCompletion>> {
    let mut information = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let buffer_size = libc::c_int::try_from(size)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process state is too large"))?;
    // SAFETY: `information` is writable storage for exactly one
    // `proc_bsdinfo`; PROC_PIDTBSDINFO writes at most `buffer_size` bytes.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            DARWIN_INCLUDE_ZOMBIES,
            information.as_mut_ptr().cast(),
            buffer_size,
        )
    };
    if read != buffer_size {
        let error = io::Error::last_os_error();
        return Err(
            if read <= 0 && error.raw_os_error().is_some_and(|code| code != 0) {
                error
            } else {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("process state returned {read} of {buffer_size} bytes"),
                )
            },
        );
    }
    // SAFETY: a full proc_bsdinfo was initialized when proc_pidinfo returned
    // its exact size.
    let information = unsafe { information.assume_init() };
    if information.pbi_pid != pid as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "process state described a different PID",
        ));
    }
    let inspected = darwin_process_snapshot_completion(
        information.pbi_status,
        information.pbi_flags,
        information.pbi_xstatus,
    );
    inspected
        .map(|inspected| {
            darwin_waitid_completion(pid).map(|waited| DarwinChildCompletion { waited, inspected })
        })
        .transpose()
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DarwinChildCompletion {
    pub(crate) waited: std::process::ExitStatus,
    pub(crate) inspected: std::process::ExitStatus,
}

#[cfg(target_os = "macos")]
fn darwin_waitid_completion(pid: libc::pid_t) -> io::Result<std::process::ExitStatus> {
    use std::os::unix::process::ExitStatusExt as _;

    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    loop {
        // PROC_PIDTBSDINFO has already reported SZOMB, so this parent-owned
        // wait is immediately satisfiable. WNOWAIT preserves the child and
        // its process-group identity until descendants have been stopped.
        // SAFETY: `information` is writable storage for one siginfo_t and
        // P_PID restricts the observation to this unreaped direct child.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                information.as_mut_ptr(),
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if result == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    // SAFETY: successful waitid initializes siginfo_t, and si_status is the
    // child exit code or terminating signal for the CLD_* cases below.
    let information = unsafe { information.assume_init() };
    if unsafe { information.si_pid() } != pid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "waitid described a different child",
        ));
    }
    let detail = unsafe { information.si_status() };
    let raw = match information.si_code {
        libc::CLD_EXITED => detail << 8,
        libc::CLD_KILLED => detail & 0x7f,
        libc::CLD_DUMPED => (detail & 0x7f) | 0x80,
        code => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("waitid reported unexpected child state {code}"),
            ));
        }
    };
    Ok(std::process::ExitStatus::from_raw(raw))
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DarwinProcessState {
    Live,
    Exiting,
    Zombie,
}

#[cfg(target_os = "macos")]
fn darwin_process_snapshot_state(status: u32, flags: u32) -> DarwinProcessState {
    if status == libc::SZOMB {
        DarwinProcessState::Zombie
    } else if flags & DARWIN_PROC_FLAG_INEXIT != 0 {
        DarwinProcessState::Exiting
    } else {
        DarwinProcessState::Live
    }
}

#[cfg(target_os = "macos")]
fn darwin_process_snapshot_completion(
    status: u32,
    flags: u32,
    raw_wait_status: u32,
) -> Option<std::process::ExitStatus> {
    use std::os::unix::process::ExitStatusExt as _;

    matches!(
        darwin_process_snapshot_state(status, flags),
        DarwinProcessState::Zombie
    )
    .then(|| std::process::ExitStatus::from_raw(raw_wait_status as i32))
}

#[cfg(not(target_os = "macos"))]
impl ChildExitObserver {
    pub(crate) fn new(_child: &Child) -> io::Result<Self> {
        Ok(Self)
    }
}
impl ChildExitObserver {
    /// The caller must retain the unreaped Child until any group cleanup ends.
    /// Never call this observer again after Child::wait has released its PID.
    pub(crate) fn completed(&self, child: &Child) -> io::Result<Option<ExitStatus>> {
        #[cfg(target_os = "macos")]
        {
            if child.id() != self.pid as u32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "observer belongs to another child",
                ));
            }
            self.completion()
                .map(|completion| completion.map(|value| value.waited))
        }
        #[cfg(not(target_os = "macos"))]
        {
            super::completed_without_reaping(child)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_observer_keeps_the_direct_child_waitable_until_explicit_reap() {
        use std::{
            io::Write,
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let mut child = Command::new("sh")
            .args(["-c", "read release; exit 7"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let before = ChildExitObserver::new(&child).unwrap();
        assert!(before.completed(&child).unwrap().is_none());
        child.stdin.take().unwrap().write_all(b"go\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = before.completed(&child).unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "child failed to exit");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(status.code(), Some(7));
        let after = ChildExitObserver::new(&child).unwrap();
        assert_eq!(after.completed(&child).unwrap(), Some(status));
        assert_eq!(before.completed(&child).unwrap(), Some(status));
        assert_eq!(child.wait().unwrap(), status);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_process_snapshot_requires_a_stable_zombie_before_cleanup() {
        use std::os::unix::process::ExitStatusExt as _;

        assert_eq!(
            darwin_process_snapshot_state(libc::SRUN, DARWIN_PROC_FLAG_INEXIT),
            DarwinProcessState::Exiting,
        );
        assert_eq!(
            darwin_process_snapshot_state(libc::SZOMB, 0),
            DarwinProcessState::Zombie,
        );
        assert_eq!(
            darwin_process_snapshot_state(libc::SRUN, 0),
            DarwinProcessState::Live,
        );
        assert!(
            darwin_process_snapshot_completion(libc::SRUN, DARWIN_PROC_FLAG_INEXIT, 23 << 8,)
                .is_none(),
            "an exiting snapshot exposed a wait status before it was stable"
        );
        assert_eq!(
            darwin_process_snapshot_completion(libc::SZOMB, 0, 0)
                .unwrap()
                .code(),
            Some(0),
        );
        assert_eq!(
            darwin_process_snapshot_completion(libc::SZOMB, 0, 23 << 8)
                .unwrap()
                .code(),
            Some(23),
        );
        assert_eq!(
            darwin_process_snapshot_completion(libc::SZOMB, 0, libc::SIGKILL as u32)
                .unwrap()
                .signal(),
            Some(libc::SIGKILL),
        );
    }
}
