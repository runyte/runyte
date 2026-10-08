// SPDX-License-Identifier: MPL-2.0

//! Bounded worker-side animation decoding and pane-local playback clocks.
use anyhow::{Result, ensure};
use gpui::RenderImage;
use image::{AnimationDecoder, ImageDecoder};
use std::{
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ANIMATION_BYTES: usize = 32 * 1024 * 1024;
const MAX_FRAMES: usize = 1024;

/// An animation covered completely by opaque overlays has nothing to advance.
pub(super) fn visible(body: runyte::layout::Rect, overlays: &[runyte::layout::Rect]) -> bool {
    if body.width == 0 || body.height == 0 {
        return false;
    }
    let right = body.x.saturating_add(body.width);
    for y in body.y..body.y.saturating_add(body.height) {
        let mut spans: Vec<_> = overlays
            .iter()
            .filter(|r| y >= r.y && y < r.y.saturating_add(r.height))
            .map(|r| (r.x, r.x.saturating_add(r.width)))
            .collect();
        spans.sort_unstable();
        let mut covered = body.x;
        for (start, end) in spans {
            if start > covered {
                break;
            }
            covered = covered.max(end);
        }
        if covered < right {
            return true;
        }
    }
    false
}

pub(super) struct Animation {
    pub frames: Vec<Arc<RenderImage>>,
    pub delays: Vec<Duration>,
    /// Total presentations of the sequence, including the first; None loops forever.
    pub plays: Option<u32>,
}
impl Animation {
    pub fn bytes(&self) -> usize {
        self.frames
            .iter()
            .map(|f| f.as_bytes(0).unwrap().len())
            .sum()
    }
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_BYTES as u64);
    limits
}

struct Source<'a> {
    file: std::fs::File,
    length: u64,
    position: u64,
    cancel: &'a AtomicBool,
}
impl<'a> Source<'a> {
    fn open(path: &Path, cancel: &'a AtomicBool) -> Result<BufReader<Self>> {
        let file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.len() <= 128 * 1024 * 1024,
            "media must be a regular file up to 128 MiB"
        );
        Ok(BufReader::new(Self {
            file,
            length: metadata.len(),
            position: 0,
            cancel,
        }))
    }
    fn check(&self) -> std::io::Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(std::io::Error::other("media decoding cancelled"));
        }
        Ok(())
    }
}
impl Read for Source<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.check()?;
        let count = buf.len().min((self.length - self.position) as usize);
        let read = self.file.read(&mut buf[..count])?;
        self.position += read as u64;
        Ok(read)
    }
}
impl Seek for Source<'_> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.check()?;
        let next = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        if !(0..=i128::from(self.length)).contains(&next) {
            return Err(std::io::Error::other("media seek outside source"));
        }
        self.position = self.file.seek(SeekFrom::Start(next as u64))?;
        Ok(self.position)
    }
}

fn webp_orientation(mut input: impl Read + Seek) -> Result<image::metadata::Orientation> {
    input.seek(SeekFrom::Start(12))?;
    loop {
        let mut chunk = [0; 8];
        if input.read(&mut chunk[..1])? == 0 {
            return Ok(image::metadata::Orientation::NoTransforms);
        }
        input.read_exact(&mut chunk[1..])?;
        let length = u32::from_le_bytes(chunk[4..].try_into().unwrap());
        if &chunk[..4] == b"EXIF" {
            ensure!(length <= 65536, "animated WebP EXIF exceeds 64 KiB limit");
            let mut bytes = vec![0; length as usize];
            input.read_exact(&mut bytes)?;
            return Ok(image::metadata::Orientation::from_exif_chunk(&bytes)
                .unwrap_or(image::metadata::Orientation::NoTransforms));
        }
        input.seek(SeekFrom::Current(i64::from(length) + i64::from(length % 2)))?;
    }
}

pub(super) fn decode(
    path: &Path,
    format: image::ImageFormat,
    cancel: &AtomicBool,
) -> Result<Option<Animation>> {
    let file = || Source::open(path, cancel);
    match format {
        image::ImageFormat::Gif => {
            let (plays, frames) = gif_info(file()?, cancel)?;
            if frames == 1 {
                return Ok(None);
            }
            ensure!(frames <= MAX_FRAMES, "animated image exceeds 1024 frames");
            let mut decoder = image::codecs::gif::GifDecoder::new(file()?)?;
            decoder.set_limits(limits())?;
            check_dimensions(decoder.dimensions())?;
            collect(
                decoder.into_frames(),
                plays,
                image::metadata::Orientation::NoTransforms,
                cancel,
                MAX_ANIMATION_BYTES,
                MAX_FRAMES,
            )
            .map(Some)
        }
        image::ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(file()?)?;
            if !decoder.has_animation() {
                return Ok(None);
            }
            decoder.set_limits(limits())?;
            check_dimensions(decoder.dimensions())?;
            let plays = match decoder.loop_count() {
                image::metadata::LoopCount::Infinite => None,
                image::metadata::LoopCount::Finite(n) => Some(n.get()),
            };
            let orientation = webp_orientation(file()?)?;
            collect(
                decoder.into_frames(),
                plays,
                orientation,
                cancel,
                MAX_ANIMATION_BYTES,
                MAX_FRAMES,
            )
            .map(Some)
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
#[path = "tests/animation.rs"]
mod tests;

fn check_dimensions((width, height): (u32, u32)) -> Result<()> {
    ensure!(
        width > 0 && height > 0 && width <= 8192 && height <= 8192,
        "image dimensions exceed 8192 pixels"
    );
    ensure!(
        u64::from(width) * u64::from(height) * 4 <= MAX_BYTES as u64,
        "decoded image exceeds 64 MiB limit"
    );
    Ok(())
}

fn collect(
    frames: image::Frames<'_>,
    plays: Option<u32>,
    orientation: image::metadata::Orientation,
    cancel: &AtomicBool,
    budget: usize,
    frame_limit: usize,
) -> Result<Animation> {
    let mut result = Animation {
        frames: Vec::new(),
        delays: Vec::new(),
        plays,
    };
    let mut bytes = 0;
    let mut frames = frames.into_iter();
    loop {
        ensure!(!cancel.load(Ordering::Acquire), "media decoding cancelled");
        let Some(frame) = frames.next() else {
            break;
        };
        let frame = frame?;
        ensure!(!cancel.load(Ordering::Acquire), "media decoding cancelled");
        let delay = Duration::from(frame.delay());
        // Browser-style protection against zero/one-centisecond busy loops.
        let delay = if delay < Duration::from_millis(20) {
            Duration::from_millis(100)
        } else {
            delay
        };
        let mut decoded = image::DynamicImage::ImageRgba8(frame.into_buffer());
        decoded.apply_orientation(orientation);
        if decoded.width() > 2048 || decoded.height() > 2048 {
            decoded = decoded.thumbnail(2048, 2048);
        }
        let mut pixels = decoded.into_rgba8();
        ensure!(
            result.frames.len() < frame_limit && pixels.len() <= budget.saturating_sub(bytes),
            "animated image exceeds decoded frame budget (32 MiB or 1024 frames)"
        );
        bytes += pixels.len();
        for pixel in pixels.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        result
            .frames
            .push(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])));
        result.delays.push(delay);
    }
    ensure!(
        !result.frames.is_empty(),
        "animated image contains no frames"
    );
    Ok(result)
}

/// GIF's Netscape count is repetitions *after* the first play. An absent
/// extension means one play (image's generic loop_count API loses that case).
fn gif_info(mut input: impl Read + Seek, cancel: &AtomicBool) -> Result<(Option<u32>, usize)> {
    fn skip(input: &mut (impl Read + Seek), count: usize) -> Result<()> {
        input.seek(SeekFrom::Current(count as i64))?;
        Ok(())
    }
    fn byte(input: &mut impl Read) -> Result<u8> {
        let mut b = [0];
        input.read_exact(&mut b)?;
        Ok(b[0])
    }
    let mut header = [0; 13];
    input.read_exact(&mut header)?;
    ensure!(
        &header[..6] == b"GIF87a" || &header[..6] == b"GIF89a",
        "invalid GIF header"
    );
    if header[10] & 0x80 != 0 {
        skip(&mut input, 3 << ((header[10] & 7) + 1))?;
    }
    let mut plays = Some(1);
    let mut frames = 0;
    loop {
        ensure!(!cancel.load(Ordering::Acquire), "media decoding cancelled");
        let application = match byte(&mut input)? {
            0x3b => return Ok((plays, frames)),
            0x2c => {
                frames += 1;
                let mut descriptor = [0; 9];
                input.read_exact(&mut descriptor)?;
                if descriptor[8] & 0x80 != 0 {
                    skip(&mut input, 3 << ((descriptor[8] & 7) + 1))?;
                }
                byte(&mut input)?; // LZW minimum code size
                false
            }
            0x21 => byte(&mut input)? == 0xff,
            _ => anyhow::bail!("invalid GIF block"),
        };
        let mut first = true;
        let mut netscape = false;
        loop {
            ensure!(!cancel.load(Ordering::Acquire), "media decoding cancelled");
            let size = byte(&mut input)? as usize;
            if size == 0 {
                break;
            }
            let mut data = [0; 255];
            input.read_exact(&mut data[..size])?;
            if first {
                netscape = application
                    && (&data[..size] == b"NETSCAPE2.0" || &data[..size] == b"ANIMEXTS1.0");
            } else if netscape && size == 3 && data[0] == 1 {
                let repeats = u16::from_le_bytes([data[1], data[2]]);
                plays = (repeats != 0).then_some(u32::from(repeats) + 1);
            }
            first = false;
        }
    }
}

#[derive(Default)]
pub(super) struct Playback {
    pub frame: usize,
    completed: u32,
    deadline: Option<Instant>,
}
impl Playback {
    pub fn suspend(&mut self) {
        self.deadline = None;
    }
    pub fn update(&mut self, animation: &Animation, now: Instant, paused: bool) -> Option<Instant> {
        if animation.frames.len() <= 1
            || paused
            || animation.plays.is_some_and(|n| self.completed >= n)
        {
            self.suspend();
            return None;
        }
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            if self.frame + 1 == animation.frames.len() {
                self.completed = self.completed.saturating_add(1);
                if animation.plays.is_some_and(|n| self.completed >= n) {
                    self.suspend();
                    return None;
                }
                self.frame = 0;
            } else {
                self.frame += 1;
            }
            self.deadline = None;
        }
        Some(
            *self
                .deadline
                .get_or_insert(now + animation.delays[self.frame]),
        )
    }
}
