// SPDX-License-Identifier: MPL-2.0

//! Platform clipboard boundary: native UTF-16 on Windows, bounded helpers on Unix.

use anyhow::Result;
#[cfg(not(windows))]
mod helpers;
#[cfg(windows)]
mod windows;
#[cfg(not(windows))]
use helpers::*;

const MAX_CLIPBOARD_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// Testable clipboard boundary used by the editor.
pub trait SystemClipboard: Send {
    fn read(&mut self) -> Result<String>;
    fn write(&mut self, text: &str) -> Result<()>;

    /// The image the clipboard holds, if it holds one.
    ///
    /// `Ok(None)` is the ordinary answer for a clipboard carrying text, a
    /// file, or nothing at all, and is distinct from an error: a paste key
    /// asks this first and falls back to text, so "no image here" must not
    /// read as "the clipboard is broken". The bytes are handed over exactly as
    /// the platform produced them, except native bitmap clipboard layouts may
    /// be encoded into a portable image format. Storage still verifies the
    /// image signature before choosing an extension.
    ///
    /// Clipboards that cannot produce an image at all default to `None` rather
    /// than to a failure, which is what keeps the inert and in-memory
    /// clipboards used by tests and the headless facade honest without each
    /// of them restating it.
    fn read_image(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// Clipboard backed by the native platform implementation.
#[derive(Default)]
pub struct CommandClipboard;

impl SystemClipboard for CommandClipboard {
    fn read(&mut self) -> Result<String> {
        #[cfg(windows)]
        {
            windows::read()
        }
        #[cfg(not(windows))]
        {
            read_with_candidates(read_candidates())
        }
    }

    fn write(&mut self, text: &str) -> Result<()> {
        #[cfg(windows)]
        {
            windows::write(text)
        }
        #[cfg(not(windows))]
        {
            write_with_candidates(write_candidates(), text)
        }
    }

    fn read_image(&mut self) -> Result<Option<Vec<u8>>> {
        #[cfg(windows)]
        {
            windows::read_image()
        }
        #[cfg(not(windows))]
        {
            read_clipboard_image()
        }
    }
}
