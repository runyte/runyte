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
}

impl Lease {
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
        bytes: Vec<u8>,
        reply: oneshot::Sender<Reply>,
    },
    Closed(u64),
    #[cfg(windows)]
    Retired {
        error: Option<String>,
    },
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
