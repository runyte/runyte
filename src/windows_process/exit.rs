// SPDX-License-Identifier: MPL-2.0

//! One-shot notification for a process returned by native launch.

use std::{
    ffi::c_void,
    io::{self, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    pin::pin,
    ptr,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::{
    runtime::Handle,
    sync::{Notify, oneshot},
};
use windows_sys::Win32::{
    Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        JobObjects::TerminateJobObject,
        Threading::{
            GetCurrentProcess, GetExitCodeProcess, INFINITE, RegisterWaitForSingleObject,
            UnregisterWaitEx, WT_EXECUTEONLYONCE,
        },
    },
};

struct Signal {
    exited: AtomicBool,
    notify: Notify,
}

struct CallbackBundle {
    process: OwnedHandle,
    job: OwnedHandle,
    _runtime: Handle,
    signal: Arc<Signal>,
    job_terminated: AtomicBool,
    notifier: Mutex<Option<oneshot::Sender<()>>>,
}

/// Watches the exact process object created by the launcher. The callback owns
/// duplicates of both the process and job handles until unregister completes.
pub(crate) struct ChildExitWatcher {
    registration: HANDLE,
    bundle: *mut CallbackBundle,
    signal: Arc<Signal>,
}

// One registration owns heap-stable callback storage. Callback mutation is
// synchronized and Windows permits unregistering from another thread.
unsafe impl Send for ChildExitWatcher {}
unsafe impl Sync for ChildExitWatcher {}

impl ChildExitWatcher {
    pub(super) fn new(process: &OwnedHandle, job: &OwnedHandle) -> io::Result<Self> {
        let runtime = Handle::try_current().map_err(|error| {
            io::Error::other(format!("no Tokio runtime for process exit: {error}"))
        })?;
        let signal = Arc::new(Signal {
            exited: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let (notifier, notified) = oneshot::channel();
        let task_signal = Arc::clone(&signal);
        runtime.spawn(async move {
            if notified.await.is_ok() {
                task_signal.notify.notify_waiters();
            }
        });
        let bundle = Box::into_raw(Box::new(CallbackBundle {
            process: duplicate(process)?,
            job: duplicate(job)?,
            _runtime: runtime,
            signal: Arc::clone(&signal),
            job_terminated: AtomicBool::new(false),
            notifier: Mutex::new(Some(notifier)),
        }));
        let mut registration = ptr::null_mut();
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut registration,
                (*bundle).process.as_raw_handle(),
                Some(on_process_exit),
                bundle.cast(),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if registered == 0 {
            let error = io::Error::last_os_error();
            unsafe {
                drop(Box::from_raw(bundle));
            }
            return Err(error);
        }
        Ok(Self {
            registration,
            bundle,
            signal,
        })
    }

    /// Waits for leader exit, captures its status, and terminates every process
    /// retained by the same private job so stdout can be drained to EOF.
    pub(crate) async fn wait(&self) -> io::Result<std::process::ExitStatus> {
        loop {
            let notified = self.signal.notify.notified();
            let mut notified = pin!(notified);
            notified.as_mut().enable();
            if self.signal.exited.load(Ordering::Acquire) {
                break;
            }
            notified.await;
        }
        let bundle = unsafe { &*self.bundle };
        let mut code = 0;
        if unsafe { GetExitCodeProcess(bundle.process.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if !bundle.job_terminated.swap(true, Ordering::AcqRel)
            && unsafe { TerminateJobObject(bundle.job.as_raw_handle(), 1) } == 0
        {
            bundle.job_terminated.store(false, Ordering::Release);
            return Err(io::Error::last_os_error());
        }
        use std::os::windows::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(code))
    }

    fn terminate_owned_job(&self) -> io::Result<()> {
        let bundle = unsafe { &*self.bundle };
        if unsafe { TerminateJobObject(bundle.job.as_raw_handle(), 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            bundle.job_terminated.store(true, Ordering::Release);
            Ok(())
        }
    }

    #[cfg(test)]
    pub(super) fn fixture_terminate_retained_job(&self) -> io::Result<()> {
        self.terminate_owned_job()
    }
}

fn duplicate(handle: &OwnedHandle) -> io::Result<OwnedHandle> {
    let mut duplicated = ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle.as_raw_handle(),
            GetCurrentProcess(),
            &mut duplicated,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(duplicated) })
}

unsafe extern "system" fn on_process_exit(context: *mut c_void, timed_out: bool) {
    if timed_out {
        return;
    }
    let bundle = unsafe { &*context.cast::<CallbackBundle>() };
    bundle.signal.exited.store(true, Ordering::Release);
    let mut sender = bundle
        .notifier
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(sender) = sender.take() {
        let _ = sender.send(());
    }
}

impl Drop for ChildExitWatcher {
    fn drop(&mut self) {
        let completed = unsafe { UnregisterWaitEx(self.registration, INVALID_HANDLE_VALUE) };
        if completed != 0 {
            unsafe {
                drop(Box::from_raw(self.bundle));
            }
        } else {
            let error = io::Error::last_os_error();
            let termination = self.terminate_owned_job();
            let _ = writeln!(
                io::stderr(),
                "native process wait unregister failed; retaining callback storage: {error}; \
                 job termination: {}",
                termination
                    .err()
                    .map_or_else(|| "accepted".into(), |error| error.to_string())
            );
        }
    }
}
