// SPDX-License-Identifier: MPL-2.0

//! Literal text admissible in a native terminal proposal. Validation
//! grants no permission to send it: physical, one-shot approval and a captured
//! live target are still required. Delivery records describe PTY writes, never
//! acceptance or execution by the child application.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// Input context captured when the person begins reviewing a proposal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputSignature {
    pub generation: u64,
    pub modes: super::emulator::Modes,
    pub alternate_screen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeliveryState {
    Queued,
    Writing,
    Delivered,
    Cancelled,
    OutcomeUnknown,
}

/// Cloneable cancellation and acknowledgment for one exact queued insertion.
#[derive(Clone, Debug)]
pub struct Delivery(Arc<AtomicU8>);
#[cfg_attr(not(unix), allow(dead_code))]
impl Delivery {
    pub(super) fn queued() -> Self {
        Self(Arc::new(AtomicU8::new(DeliveryState::Queued as u8)))
    }
    #[cfg(test)]
    pub(crate) fn queued_for_test() -> Self {
        Self::queued()
    }
    #[cfg(test)]
    pub(crate) fn claim_for_test(&self) -> bool {
        self.claim()
    }

    pub fn state(&self) -> DeliveryState {
        match self.0.load(Ordering::Acquire) {
            0 => DeliveryState::Queued,
            1 => DeliveryState::Writing,
            2 => DeliveryState::Delivered,
            3 => DeliveryState::Cancelled,
            _ => DeliveryState::OutcomeUnknown,
        }
    }
    /// True means the writer cannot claim this insertion. Once writing starts,
    /// cancellation cannot establish how much the child has already received.
    pub fn cancel(&self) -> bool {
        self.transition(DeliveryState::Queued, DeliveryState::Cancelled)
    }
    pub(super) fn claim(&self) -> bool {
        self.transition(DeliveryState::Queued, DeliveryState::Writing)
    }
    pub(super) fn complete(&self) {
        self.transition(DeliveryState::Writing, DeliveryState::Delivered);
    }
    pub(super) fn abandoned(&self) {
        self.cancel();
        self.transition(DeliveryState::Writing, DeliveryState::OutcomeUnknown);
    }
    fn transition(&self, from: DeliveryState, to: DeliveryState) -> bool {
        self.0
            .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

pub const MAX_TEXT_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextError {
    Empty,
    TooLong,
    ControlOrLineBreak,
}

/// One bounded line, with no submit key, controls or injected paste framing.
/// Ordinary characters can still trigger actions in arbitrary terminal apps.
#[derive(Clone, Eq, PartialEq)]
pub struct Text(String);

#[cfg_attr(not(unix), allow(dead_code))]
impl Text {
    pub fn new(text: &str) -> Result<Self, TextError> {
        if text.is_empty() {
            return Err(TextError::Empty);
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(TextError::TooLong);
        }
        if text
            .chars()
            .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
        {
            return Err(TextError::ControlOrLineBreak);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Text {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalProposalText")
            .field("bytes", &self.0.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "tests/proposal.rs"]
mod tests;
