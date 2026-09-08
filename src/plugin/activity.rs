// SPDX-License-Identifier: MPL-2.0

//! Explicit, bounded protection for continuing application activity.
use super::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const MAX_LEASES: usize = 2;
pub const MAX_SECONDS: u64 = 600;
pub const LEASE_CHARGE: usize = 1024;
pub const CLEANUP_SECONDS: u64 = 2;
pub fn default_seconds() -> u64 {
    MAX_SECONDS
}

pub fn validate_duration(seconds: u64) -> Result<(), Error> {
    if !(1..=MAX_SECONDS).contains(&seconds) {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            "Activity duration must be 1–600 seconds",
        ));
    }
    Ok(())
}
pub fn validate_title(title: &str) -> Result<(), Error> {
    if title.is_empty() || title.len() > 160 || title.chars().any(char::is_control) {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid activity title",
        ));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Active,
    Cancelling,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub lease: String,
    pub title: String,
    pub state: State,
    pub duration_seconds: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Expired,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cancelled {
    pub lease: String,
    pub reason: Reason,
}

pub(crate) struct Lease {
    pub info: Info,
    pub deadline: Instant,
}
impl Lease {
    pub fn cancel(&mut self, reason: Reason, now: Instant) -> Option<Cancelled> {
        if self.info.state == State::Cancelling {
            return None;
        }
        self.info.state = State::Cancelling;
        self.deadline = now + Duration::from_secs(CLEANUP_SECONDS);
        Some(Cancelled {
            lease: self.info.lease.clone(),
            reason,
        })
    }
}
