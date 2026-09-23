// SPDX-License-Identifier: MPL-2.0

//! Opt-in scoped access to live workspace context, separate from frontend control.

#[cfg(any(unix, windows))]
pub mod storage;
#[cfg(any(unix, windows))]
pub mod transport;
pub mod wire;

#[cfg(any(unix, windows))]
pub mod discovery;

#[cfg(any(unix, windows))]
pub use transport::Event;
#[cfg(not(any(unix, windows)))]
pub type Event = std::convert::Infallible;
