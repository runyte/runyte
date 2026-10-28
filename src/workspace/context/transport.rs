// SPDX-License-Identifier: MPL-2.0

//! Owner-private, bounded external context transport. Admission and semantic
//! dispatch stay on the workspace thread; socket tasks never inspect App.

use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::{Notify, oneshot};

pub const FRAME_BYTES: usize = super::wire::MAX_FRAME_BYTES;
pub const CONNECTIONS: usize = 8;
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

#[derive(Default, Debug)]
pub struct Lease {
    cancelled: AtomicBool,
    wake: Notify,
    notifications: AtomicBool,
    notification_pending: AtomicBool,
    notification_outstanding: AtomicBool,
    notification_wake: Notify,
}

impl Lease {
    pub fn enable_notifications(&self, enabled: bool) {
        self.notifications.store(enabled, Ordering::Release);
        if !enabled {
            self.clear_notification();
        }
    }
    pub fn notifications_enabled(&self) -> bool {
        self.notifications.load(Ordering::Acquire)
    }
    pub fn clear_notification(&self) {
        self.notification_pending.store(false, Ordering::Release);
        self.notification_outstanding
            .store(false, Ordering::Release);
    }
    pub fn queue_notification(&self) -> &'static str {
        if !self.active() || !self.notifications_enabled() {
            return "unavailable";
        }
        if self.notification_outstanding.swap(true, Ordering::AcqRel) {
            return "already_pending";
        }
        self.notification_pending.store(true, Ordering::Release);
        self.notification_wake.notify_one();
        "queued"
    }
    async fn notification(&self) {
        loop {
            let notified = self.notification_wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active()
                && self.notifications_enabled()
                && self.notification_pending.swap(false, Ordering::AcqRel)
            {
                return;
            }
            notified.await;
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
    pub fn active(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire)
    }
    pub async fn cancelled(&self) {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.active() {
                return;
            }
            notified.await;
        }
    }
}

pub struct Reply {
    pub value: Value,
    pub lease: Option<Arc<Lease>>,
    pub close: bool,
}

pub enum Event {
    Frame {
        connection: u64,
        /// The connecting process, where the platform reports it.
        peer: Option<u32>,
        bytes: Vec<u8>,
        reply: oneshot::Sender<Reply>,
    },
    Closed(u64),
    #[cfg(windows)]
    Retired {
        error: Option<String>,
    },
}

pub(super) async fn notification(lease: &Option<Arc<Lease>>) {
    match lease {
        Some(lease) => lease.notification().await,
        None => std::future::pending().await,
    }
}

pub(super) async fn revoked(lease: &Option<Arc<Lease>>) {
    match lease {
        Some(lease) => lease.cancelled().await,
        None => std::future::pending().await,
    }
}

#[cfg(unix)]
#[path = "transport/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "transport/windows.rs"]
mod platform;
pub use platform::Server;

#[cfg(all(test, windows))]
pub(crate) use platform::NativeConnection;
#[cfg(windows)]
pub(crate) use platform::connect;

#[cfg(all(test, unix))]
#[path = "tests/transport.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "tests/transport_windows.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/notifications.rs"]
mod notification_tests;
