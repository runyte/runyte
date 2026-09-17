// SPDX-License-Identifier: MPL-2.0

//! Literal text admissible in a future native terminal proposal. Validation
//! grants no permission to send it: physical, one-shot approval and a captured
//! live target are still required. This module has no PTY input operation.

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
