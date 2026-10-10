// SPDX-License-Identifier: MPL-2.0
//! Product identity and the shared wording for edition/front-end requirements.

/// The product supplied by the executable that starts the shared CLI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Edition {
    #[default]
    Terminal,
    Desktop,
}

pub(crate) const TERMINAL_WINDOW: &str = "--window is part of the Runyte desktop edition; this is the terminal edition. See https://github.com/runyte/runyte#editions";
pub(crate) const WINDOWS_WINDOW: &str =
    "The desktop edition's window is not available on Windows yet";
pub(crate) const PREVIEW_WINDOW: &str =
    ":preview needs the Runyte window: use the desktop edition with --window";

pub(crate) const MEDIA_WINDOW: &str =
    "Media viewing needs the Runyte window: use the desktop edition with --window";

impl Edition {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Desktop => "desktop",
        }
    }
    pub(crate) fn window_refusal(self, windows: bool) -> Option<&'static str> {
        match (self, windows) {
            (Self::Terminal, _) => Some(TERMINAL_WINDOW),
            (Self::Desktop, true) => Some(WINDOWS_WINDOW),
            (Self::Desktop, false) => None,
        }
    }
    pub(crate) fn help_footer(self) -> &'static str {
        match self {
            Self::Terminal => {
                "Terminal edition. For the desktop edition, see https://github.com/runyte/runyte#editions"
            }
            Self::Desktop => "Desktop edition.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_support_depends_on_edition_and_platform() {
        assert_eq!(
            Edition::Terminal.window_refusal(false),
            Some(TERMINAL_WINDOW)
        );
        assert_eq!(
            Edition::Terminal.window_refusal(true),
            Some(TERMINAL_WINDOW)
        );
        assert_eq!(Edition::Desktop.window_refusal(true), Some(WINDOWS_WINDOW));
        assert_eq!(Edition::Desktop.window_refusal(false), None);
    }
}
