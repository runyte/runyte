// SPDX-License-Identifier: MPL-2.0

//! One-time foreground parent observation. ToolHelp supplies only a candidate
//! PID; the retained same-account handle and creation order supply identity.
//! Detached hosts have a short-lived synthetic inheritance parent and must not
//! use that process as a supervisor.

use std::{
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::Arc,
};

use windows_sys::Win32::{
    Foundation::{ERROR_BAD_LENGTH, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    },
};

use super::windows_process_identity::{PinnedProcess, ProcessIdentity};

const SNAPSHOT_ATTEMPTS: usize = 3;
const MAX_PROCESS_ROWS: usize = 65_536;

/// The caller's role is explicit so detached startup cannot mistake its
/// disposable `InheritanceParent` for a natural supervising parent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParentRole {
    Foreground,
    DetachedHost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParentUnavailable {
    DetachedHost,
    CurrentProcessNotListed,
    NoParentId,
    ParentExitedBeforePin,
}

#[derive(Debug)]
pub struct RetainedParent {
    child: ProcessIdentity,
    parent: Arc<PinnedProcess>,
}

impl RetainedParent {
    pub fn child(&self) -> ProcessIdentity {
        self.child
    }

    pub fn parent(&self) -> &Arc<PinnedProcess> {
        &self.parent
    }
}

#[derive(Debug)]
pub enum ParentObservation {
    Retained(RetainedParent),
    Unavailable(ParentUnavailable),
}

/// Capture a foreground process's candidate parent once. An unavailable
/// parent is explicit and cannot accidentally become a PID-only supervisor.
/// OS snapshot/query ambiguity is an error. No caller should retry a different
/// candidate after this observation.
pub fn observe_parent(role: ParentRole) -> io::Result<ParentObservation> {
    if role == ParentRole::DetachedHost {
        return Ok(ParentObservation::Unavailable(
            ParentUnavailable::DetachedHost,
        ));
    }
    let child = ProcessIdentity::current()?;
    let Some(candidate) = candidate_parent_pid(child.pid)? else {
        return Ok(ParentObservation::Unavailable(
            ParentUnavailable::CurrentProcessNotListed,
        ));
    };
    pin_candidate(child, candidate)
}

fn candidate_parent_pid(child_pid: u32) -> io::Result<Option<u32>> {
    let mut attempts = 0;
    let raw = loop {
        attempts += 1;
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if raw != INVALID_HANDLE_VALUE {
            break raw;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_BAD_LENGTH as i32) || attempts == SNAPSHOT_ATTEMPTS {
            return Err(error);
        }
    };
    // SAFETY: CreateToolhelp32Snapshot returned a real, owned snapshot handle.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut row = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    };
    if unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut row) } == 0 {
        return end_or_error().map(|_| None);
    }
    for _ in 0..MAX_PROCESS_ROWS {
        if row.th32ProcessID == child_pid {
            return Ok(Some(row.th32ParentProcessID));
        }
        if unsafe { Process32NextW(snapshot.as_raw_handle(), &mut row) } == 0 {
            return end_or_error().map(|_| None);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "native process snapshot exceeded its row limit",
    ))
}

fn end_or_error() -> io::Result<()> {
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
        Ok(())
    } else {
        Err(error)
    }
}

fn pin_candidate(child: ProcessIdentity, candidate: u32) -> io::Result<ParentObservation> {
    if candidate == 0 {
        return Ok(ParentObservation::Unavailable(
            ParentUnavailable::NoParentId,
        ));
    }
    if candidate == child.pid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native parent candidate names the child itself",
        ));
    }
    let parent = match PinnedProcess::open_parent_candidate(candidate)? {
        Some(parent) => parent,
        None => {
            return Ok(ParentObservation::Unavailable(
                ParentUnavailable::ParentExitedBeforePin,
            ));
        }
    };
    if parent.identity().creation_time > child.creation_time {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native parent candidate was created after this process",
        ));
    }
    Ok(ParentObservation::Retained(RetainedParent {
        child,
        parent: Arc::new(parent),
    }))
}

#[cfg(test)]
mod tests;
