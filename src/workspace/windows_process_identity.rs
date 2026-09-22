// SPDX-License-Identifier: MPL-2.0

//! Native process identity for private persistent-session publications.
//!
//! Metadata identifies a candidate, never authority to terminate it. Future
//! transport admission must also verify the actual pipe peer against this
//! retained process handle.

use std::{
    io,
    os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
    ptr,
};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{
        ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{
        EqualSid, GetTokenInformation, IsValidSid, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    },
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// Exact creation FILETIME, not a boot identifier or a display timestamp.
    pub creation_time: u64,
}

impl ProcessIdentity {
    pub fn current() -> io::Result<Self> {
        // The pseudo-handle remains valid for the current process and is not
        // transferred to an OwnedHandle.
        identity_for_handle(unsafe { GetCurrentProcess() }, std::process::id())
    }

    pub fn validate(self) -> io::Result<()> {
        if self.pid == 0 || self.creation_time == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid native process identity",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum PinResult {
    Pinned(PinnedProcess),
    Gone,
    Reused,
}

#[derive(Debug)]
pub struct PinnedProcess {
    identity: ProcessIdentity,
    process: OwnedHandle,
}

impl PinnedProcess {
    /// Pins a live same-account peer identified by the actual connected pipe.
    /// Creation time and owner are read from this one retained process handle;
    /// this does not grant authority to terminate it or reopen a later PID.
    pub fn open_peer(pid: u32) -> io::Result<Self> {
        if pid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid native pipe peer PID",
            ));
        }
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let identity = identity_for_handle(process.as_raw_handle(), pid)?;
        let current_user = User::for_process(unsafe { GetCurrentProcess() })?;
        let candidate_user = User::for_process(process.as_raw_handle())?;
        require_same_sid(current_user.sid(), candidate_user.sid())?;
        if !process_is_alive(process.as_raw_handle())? {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "native pipe peer has exited",
            ));
        }
        Ok(Self { identity, process })
    }

    pub fn open(identity: ProcessIdentity) -> io::Result<PinResult> {
        identity.validate()?;
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                identity.pid,
            )
        };
        if raw.is_null() {
            return classify_open_error(io::Error::last_os_error());
        }
        // OpenProcess returned one owned, non-inheritable handle. Every check
        // below uses that same process object, including observations after exit.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        if !process_is_alive(process.as_raw_handle())? {
            return Ok(PinResult::Gone);
        }
        if identity_for_handle(process.as_raw_handle(), identity.pid)? != identity {
            return Ok(PinResult::Reused);
        }
        let current_user = User::for_process(unsafe { GetCurrentProcess() })?;
        let candidate_user = User::for_process(process.as_raw_handle())?;
        require_same_sid(current_user.sid(), candidate_user.sid())?;
        Ok(PinResult::Pinned(Self { identity, process }))
    }

    pub fn identity(&self) -> ProcessIdentity {
        self.identity
    }

    pub fn is_alive(&self) -> io::Result<bool> {
        process_is_alive(self.process.as_raw_handle())
    }
}

impl AsHandle for PinnedProcess {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        self.process.as_handle()
    }
}

fn classify_open_error(error: io::Error) -> io::Result<PinResult> {
    // PID zero has already been rejected. Access denial and every other
    // ambiguous failure remain errors; they never authorize stale cleanup.
    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        Ok(PinResult::Gone)
    } else {
        Err(error)
    }
}

fn identity_for_handle(process: HANDLE, pid: u32) -> io::Result<ProcessIdentity> {
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let identity = ProcessIdentity {
        pid,
        creation_time: (u64::from(creation.dwHighDateTime) << 32)
            | u64::from(creation.dwLowDateTime),
    };
    identity.validate()?;
    Ok(identity)
}

fn process_is_alive(process: HANDLE) -> io::Result<bool> {
    classify_wait(
        unsafe { WaitForSingleObject(process, 0) },
        io::Error::last_os_error,
    )
}

fn classify_wait(result: u32, last_error: impl FnOnce() -> io::Error) -> io::Result<bool> {
    match result {
        WAIT_OBJECT_0 => Ok(false),
        WAIT_TIMEOUT => Ok(true),
        WAIT_FAILED => Err(last_error()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected native process wait result",
        )),
    }
}

/// TOKEN_USER embeds a SID pointer into this aligned allocation. Its storage
/// outlives every comparison and moves without relocating the allocation.
struct User(Vec<usize>);

impl User {
    fn for_process(process: HANDLE) -> io::Result<Self> {
        let mut token = ptr::null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut length = 0;
        let first = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                ptr::null_mut(),
                0,
                &mut length,
            )
        };
        if first != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected native token size result",
            ));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error);
        }
        if !(size_of::<TOKEN_USER>()..=4096).contains(&(length as usize)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid native token user length",
            ));
        }
        let capacity = length;
        let mut storage = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                capacity,
                &mut length,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if length < size_of::<TOKEN_USER>() as u32 || length > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid native token user result",
            ));
        }
        Ok(Self(storage))
    }

    fn sid(&self) -> PSID {
        // A successful GetTokenInformation initialized this TOKEN_USER and SID.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

fn require_same_sid(expected: PSID, actual: PSID) -> io::Result<()> {
    // Both non-null pointers come from live native token buffers (or native SID
    // fixture storage), retained by the caller through this comparison.
    if expected.is_null()
        || actual.is_null()
        || unsafe { IsValidSid(expected) } == 0
        || unsafe { IsValidSid(actual) } == 0
        || unsafe { EqualSid(expected, actual) } == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "native process belongs to a different user",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
