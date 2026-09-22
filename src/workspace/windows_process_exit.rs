// SPDX-License-Identifier: MPL-2.0

//! One-shot exit notification for a retained native process object.
//!
//! The registration owns the pinned handle and callback storage until a
//! completed unregister. No PID is reopened and an idle editor owns no timer.

use std::{
    ffi::c_void,
    io::{self, Write},
    os::windows::io::{AsHandle, AsRawHandle},
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
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::Threading::{
        INFINITE, RegisterWaitForSingleObject, UnregisterWaitEx, WT_EXECUTEONLYONCE,
    },
};

use super::windows_process_identity::PinnedProcess;

struct Signal {
    exited: AtomicBool,
    notify: Notify,
}

struct CallbackBundle {
    // The registered object and callback context must survive one another.
    _process: Arc<PinnedProcess>,
    // Retained for ownership provenance. A Handle alone does not keep its
    // runtime alive, so the callback never calls it after registration.
    _runtime: Handle,
    signal: Arc<Signal>,
    notifier: Mutex<Option<oneshot::Sender<()>>>,
    #[cfg(test)]
    callback_hook: Mutex<Option<Arc<tests::CallbackHook>>>,
}

/// Waits for the original process object, including after its PID is reused.
/// Dropping the watcher blocks until all native callbacks have returned. An
/// active waiter needs the captured runtime running to receive notification;
/// shutdown before callback remains safe for teardown, but cancels delivery.
pub(super) struct ProcessExitWatcher {
    registration: HANDLE,
    bundle: *mut CallbackBundle,
    signal: Arc<Signal>,
}

// The registration has one owner. Its bundle is heap-stable and callback
// mutation is synchronized; Windows permits unregistering on another thread.
unsafe impl Send for ProcessExitWatcher {}
unsafe impl Sync for ProcessExitWatcher {}

impl ProcessExitWatcher {
    pub(super) fn new(process: Arc<PinnedProcess>) -> io::Result<Self> {
        Self::new_with_registration(process, |process, context| {
            let mut registration = ptr::null_mut();
            let registered = unsafe {
                RegisterWaitForSingleObject(
                    &mut registration,
                    process.as_handle().as_raw_handle(),
                    Some(on_process_exit),
                    context,
                    INFINITE,
                    WT_EXECUTEONLYONCE,
                )
            };
            if registered == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(registration)
            }
        })
    }

    fn new_with_registration(
        process: Arc<PinnedProcess>,
        register: impl FnOnce(&PinnedProcess, *const c_void) -> io::Result<HANDLE>,
    ) -> io::Result<Self> {
        let runtime = Handle::try_current().map_err(|error| {
            io::Error::other(format!("no Tokio runtime for process exit: {error}"))
        })?;
        let signal = Arc::new(Signal {
            exited: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let (notifier, notified) = oneshot::channel();
        // This is the sole notification task. A callback's oneshot send wakes
        // only this Tokio task, never an arbitrary caller waker that could
        // reentrantly drop the watcher inside UnregisterWaitEx. Spawning now
        // also makes callback delivery safe if the runtime later shuts down.
        let task_signal = Arc::clone(&signal);
        runtime.spawn(async move {
            if notified.await.is_ok() {
                task_signal.notify.notify_waiters();
            }
        });
        let bundle = Box::into_raw(Box::new(CallbackBundle {
            _process: process,
            _runtime: runtime,
            signal: Arc::clone(&signal),
            notifier: Mutex::new(Some(notifier)),
            #[cfg(test)]
            callback_hook: Mutex::new(None),
        }));
        let registration =
            match register(unsafe { &(*bundle)._process }, bundle.cast_const().cast()) {
                Ok(registration) => registration,
                Err(error) => {
                    // A failed registration cannot have scheduled the callback.
                    unsafe {
                        drop(Box::from_raw(bundle));
                    }
                    return Err(error);
                }
            };
        Ok(Self {
            registration,
            bundle,
            signal,
        })
    }

    pub(super) async fn wait(&self) {
        loop {
            // Register the waiter first, then Acquire the sticky exit bit.
            // This closes the callback-before-wait and cancellation races.
            let notified = self.signal.notify.notified();
            let mut notified = pin!(notified);
            notified.as_mut().enable();
            if self.signal.exited.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    #[cfg(test)]
    fn has_exited(&self) -> bool {
        self.signal.exited.load(Ordering::Acquire)
    }
}

unsafe extern "system" fn on_process_exit(context: *mut c_void, timed_out: bool) {
    if timed_out {
        return; // INFINITE registration has no timeout path.
    }
    let bundle = unsafe { &*context.cast::<CallbackBundle>() };
    #[cfg(test)]
    {
        let hook = bundle
            .callback_hook
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            let _ = hook.entered.send(());
            let _ = hook
                .release
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv();
        }
    }
    bundle.signal.exited.store(true, Ordering::Release);
    let mut sender = bundle
        .notifier
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(sender) = sender.take() {
        let _ = sender.send(());
    }
}

impl Drop for ProcessExitWatcher {
    fn drop(&mut self) {
        let completed = unsafe { UnregisterWaitEx(self.registration, INVALID_HANDLE_VALUE) };
        if completed != 0 {
            // The blocking completion proves no callback still touches bundle.
            unsafe {
                drop(Box::from_raw(self.bundle));
            }
        } else {
            // Preserve the process handle, runtime provenance and callback
            // storage rather than freeing memory a pending callback may read.
            // This unexpected OS failure intentionally leaks one bundle.
            let error = io::Error::last_os_error();
            let _ = writeln!(
                io::stderr(),
                "native process wait unregister failed; retaining callback storage: {error}"
            );
        }
    }
}

#[cfg(test)]
mod tests;
