// SPDX-License-Identifier: MPL-2.0

//! Retain the native clipboard owner for the host's lifetime. In particular,
//! X11 selections disappear if the connection used to write them is dropped.

use super::{MAX_CLIPBOARD_TEXT_BYTES, SystemClipboard};
use anyhow::{Context, Result, ensure};

#[derive(Default)]
pub struct NativeClipboard {
    clipboard: Option<arboard::Clipboard>,
}
impl NativeClipboard {
    fn clipboard(&mut self) -> Result<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new().context("open system clipboard")?);
        }
        Ok(self.clipboard.as_mut().unwrap())
    }
}
impl SystemClipboard for NativeClipboard {
    fn read(&mut self) -> Result<String> {
        let text = match self.clipboard()?.get_text() {
            Ok(text) => text,
            Err(arboard::Error::ContentNotAvailable) => return Ok(String::new()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            text.len() <= MAX_CLIPBOARD_TEXT_BYTES,
            "clipboard text exceeds 64 MiB"
        );
        Ok(text)
    }
    fn write(&mut self, text: &str) -> Result<()> {
        ensure!(
            text.len() <= MAX_CLIPBOARD_TEXT_BYTES,
            "clipboard text exceeds 64 MiB"
        );
        self.clipboard()?.set_text(text)?;
        Ok(())
    }
    fn read_image(&mut self) -> Result<Option<Vec<u8>>> {
        // Rich browser selections may offer text and an image. Match the
        // existing clipboard contract: available text takes precedence.
        match self.clipboard()?.get_text() {
            Ok(_) => return Ok(None),
            Err(arboard::Error::ContentNotAvailable) => {}
            Err(error) => return Err(error.into()),
        }
        let image = match self.clipboard()?.get_image() {
            Ok(image) => image,
            Err(arboard::Error::ContentNotAvailable) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            image.bytes.len() <= MAX_CLIPBOARD_TEXT_BYTES,
            "clipboard image exceeds 64 MiB"
        );
        let width = u32::try_from(image.width)?;
        let height = u32::try_from(image.height)?;
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(
            &image.bytes,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )?;
        ensure!(
            png.len() <= MAX_CLIPBOARD_TEXT_BYTES,
            "clipboard PNG exceeds 64 MiB"
        );
        Ok(Some(png))
    }
}
