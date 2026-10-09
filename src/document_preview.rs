// SPDX-License-Identifier: MPL-2.0
//! Bounded, engine-independent capture for the experimental native renderer.
pub const MAX_BYTES: usize = 128 * 1024;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentPreview {
    pub generation: u64,
    pub source: usize,
    pub text: String,
    pub selection_only: bool,
    pub language: String,
    pub path: Option<std::path::PathBuf>,
}

/// Capture already-resolved editor spans. Empty spans mean no explicit selection.
/// The caller supplies the same inclusive/half-open semantics used by yank.
pub fn capture(
    buffer: &crate::buffer::Buffer,
    spans: &[(usize, usize)],
) -> Result<(String, bool), &'static str> {
    if spans.is_empty() {
        if buffer.len_bytes() > MAX_BYTES {
            return Err("preview prototype limit is 128 KiB; select a smaller section");
        }
        return Ok((buffer.to_string(), false));
    }
    if spans.len() > 256 {
        return Err("preview prototype limit is 256 selections");
    }
    let mut text = String::new();
    for &(from, to) in spans {
        if to.saturating_sub(from) > MAX_BYTES {
            return Err("selected preview exceeds 128 KiB");
        }
        let part = buffer.slice(from, to);
        let separator = usize::from(!text.is_empty());
        if text.len() + part.len() + separator > MAX_BYTES {
            return Err("selected preview exceeds 128 KiB");
        }
        if separator != 0 {
            text.push('\n');
        }
        text.push_str(&part);
    }
    Ok((text, true))
}
