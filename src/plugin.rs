// SPDX-License-Identifier: MPL-2.0

//! Versioned external-process extension boundary; bundled frontend DTOs stay private.

pub mod activity;
pub mod application;
pub mod arguments;
pub mod editor;
mod epoch1;
pub mod filesystem;
pub mod handoff;
pub mod interaction;
pub mod observation;
pub mod process;
pub mod provider;
pub(crate) mod staging;
pub mod view;
mod worker;
pub use epoch1::*;
pub use worker::*;
