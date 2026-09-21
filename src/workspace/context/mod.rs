// SPDX-License-Identifier: MPL-2.0

//! Opt-in scoped access to live workspace context, separate from frontend control.

#[cfg(unix)]
pub mod storage;
#[cfg(unix)]
pub mod transport;
pub mod wire;

#[cfg(unix)]
pub mod discovery;

#[cfg(unix)]
pub use transport::Event;
#[cfg(not(unix))]
pub type Event = std::convert::Infallible;
