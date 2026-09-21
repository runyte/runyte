// SPDX-License-Identifier: MPL-2.0

//! A bounded packed-DIB reader. No GDI objects, color-profile files, or codecs
//! are opened while interpreting clipboard-controlled bytes.

use anyhow::{Context, Result, ensure};
use std::io::{self, Write};

const LIMIT: usize = crate::pasted_image::MAX_IMAGE_BYTES;
const RGB: u32 = 0;
const BITFIELDS: u32 = 3;
const SRGB: u32 = 0x7352_4742;
const WINDOWS_COLOR_SPACE: u32 = 0x5769_6e20;

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .context("truncated DIB header")?
            .try_into()
            .unwrap(),
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated DIB header")?
            .try_into()
            .unwrap(),
    ))
}

#[derive(Clone, Copy)]
struct Channel {
    mask: u32,
    shift: u32,
    maximum: u32,
}

impl Channel {
    fn new(mask: u32, bits: u16) -> Result<Self> {
        ensure!(mask != 0, "DIB color mask is empty");
        ensure!(
            bits == 32 || mask >> bits == 0,
            "DIB mask exceeds pixel width"
        );
        let shift = mask.trailing_zeros();
        let maximum = mask >> shift;
        ensure!(
            maximum & maximum.wrapping_add(1) == 0,
            "DIB color mask is not contiguous"
        );
        Ok(Self {
            mask,
            shift,
            maximum,
        })
    }

    fn extract(self, value: u32) -> u8 {
        let component = ((value & self.mask) >> self.shift) as u64;
        ((component * 255 + u64::from(self.maximum) / 2) / u64::from(self.maximum)) as u8
    }
}

enum Pixels<'a> {
    Indexed(&'a [u8]),
    Bgr,
    Masked([Channel; 3], Option<Channel>),
}

struct Dib<'a> {
    bytes: &'a [u8],
    width: usize,
    height: usize,
    bits: u16,
    top_down: bool,
    offset: usize,
    stride: usize,
    pixels: Pixels<'a>,
}

impl<'a> Dib<'a> {
    fn parse(bytes: &'a [u8], require_v5: bool) -> Result<Self> {
        ensure!(bytes.len() <= LIMIT, "clipboard bitmap exceeds 64 MiB");
        let header = u32_at(bytes, 0)? as usize;
        ensure!(matches!(header, 40 | 108 | 124), "unsupported DIB header");
        ensure!(!require_v5 || header == 124, "CF_DIBV5 has no V5 header");
        ensure!(bytes.len() >= header, "truncated DIB header");
        let width = u32_at(bytes, 4)? as i32;
        let signed_height = u32_at(bytes, 8)? as i32;
        ensure!(width > 0 && signed_height != 0, "invalid DIB dimensions");
        let height = signed_height.checked_abs().context("invalid DIB height")? as usize;
        let width = width as usize;
        let decoded = width.checked_mul(height).and_then(|n| n.checked_mul(4));
        ensure!(
            decoded.is_some_and(|n| n <= LIMIT),
            "decoded clipboard bitmap exceeds 64 MiB"
        );
        ensure!(u16_at(bytes, 12)? == 1, "DIB must have one plane");
        let bits = u16_at(bytes, 14)?;
        ensure!(
            matches!(bits, 1 | 4 | 8 | 16 | 24 | 32),
            "unsupported DIB pixel width"
        );
        let compression = u32_at(bytes, 16)?;
        ensure!(
            matches!(compression, RGB | BITFIELDS),
            "compressed DIB images are unsupported"
        );
        ensure!(
            compression != BITFIELDS || matches!(bits, 16 | 32),
            "invalid DIB bitfields pixel width"
        );
        if header >= 108 {
            ensure!(
                matches!(u32_at(bytes, 56)?, SRGB | WINDOWS_COLOR_SPACE),
                "DIB color profiles and calibrated colors are unsupported"
            );
        }
        if header == 124 {
            ensure!(
                u32_at(bytes, 112)? == 0 && u32_at(bytes, 116)? == 0,
                "DIB color profiles are unsupported"
            );
            ensure!(u32_at(bytes, 120)? == 0, "invalid DIB reserved field");
        }
        let mut offset = header;
        let masks = if compression == BITFIELDS {
            if header == 40 {
                offset += 12;
            }
            Some([
                u32_at(bytes, 40)?,
                u32_at(bytes, 44)?,
                u32_at(bytes, 48)?,
                if header >= 108 { u32_at(bytes, 52)? } else { 0 },
            ])
        } else {
            match bits {
                16 => Some([
                    0x7c00,
                    0x03e0,
                    0x001f,
                    if header >= 108 { u32_at(bytes, 52)? } else { 0 },
                ]),
                32 => Some([
                    0x00ff_0000,
                    0x0000_ff00,
                    0x0000_00ff,
                    if header >= 108 { u32_at(bytes, 52)? } else { 0 },
                ]),
                _ => None,
            }
        };
        let used = u32_at(bytes, 32)? as usize;
        let colors = if bits <= 8 {
            ensure!(
                used <= 1usize << bits,
                "DIB palette exceeds pixel index range"
            );
            if used == 0 { 1usize << bits } else { used }
        } else {
            used
        };
        let palette_end = colors
            .checked_mul(4)
            .and_then(|n| offset.checked_add(n))
            .context("DIB palette size overflow")?;
        let palette = bytes
            .get(offset..palette_end)
            .context("truncated DIB palette")?;
        offset = palette_end;
        let pixels = if bits <= 8 {
            Pixels::Indexed(palette)
        } else if bits == 24 {
            Pixels::Bgr
        } else {
            let masks = masks.unwrap();
            for (index, mask) in masks.iter().enumerate() {
                ensure!(
                    masks[..index].iter().all(|prior| prior & mask == 0),
                    "DIB color masks overlap"
                );
            }
            Pixels::Masked(
                [
                    Channel::new(masks[0], bits)?,
                    Channel::new(masks[1], bits)?,
                    Channel::new(masks[2], bits)?,
                ],
                if masks[3] == 0 {
                    None
                } else {
                    Some(Channel::new(masks[3], bits)?)
                },
            )
        };
        let stride = width
            .checked_mul(usize::from(bits))
            .and_then(|n| n.checked_add(31))
            .map(|n| (n / 32) * 4)
            .context("DIB stride overflow")?;
        let size = stride
            .checked_mul(height)
            .context("DIB image size overflow")?;
        ensure!(
            offset
                .checked_add(size)
                .is_some_and(|end| end <= bytes.len()),
            "truncated DIB pixels"
        );
        let advertised = u32_at(bytes, 20)? as usize;
        ensure!(
            advertised == 0
                || (advertised >= size
                    && offset
                        .checked_add(advertised)
                        .is_some_and(|end| end <= bytes.len())),
            "invalid DIB image size"
        );
        Ok(Self {
            bytes,
            width,
            height,
            bits,
            top_down: signed_height < 0,
            offset,
            stride,
            pixels,
        })
    }

    fn row(&self, y: usize, target: &mut [u8]) -> Result<()> {
        let y = if self.top_down {
            y
        } else {
            self.height - y - 1
        };
        let start = self.offset + y * self.stride;
        let source = &self.bytes[start..start + self.stride];
        for (x, rgba) in target.chunks_exact_mut(4).enumerate() {
            match &self.pixels {
                Pixels::Indexed(palette) => {
                    let index = match self.bits {
                        1 => (source[x / 8] >> (7 - x % 8)) & 1,
                        4 => (source[x / 2] >> (if x % 2 == 0 { 4 } else { 0 })) & 15,
                        8 => source[x],
                        _ => unreachable!(),
                    } as usize;
                    let color = palette
                        .get(index * 4..index * 4 + 4)
                        .context("DIB pixel index exceeds its palette")?;
                    rgba.copy_from_slice(&[color[2], color[1], color[0], 255]);
                }
                Pixels::Bgr => {
                    let color = &source[x * 3..x * 3 + 3];
                    rgba.copy_from_slice(&[color[2], color[1], color[0], 255]);
                }
                Pixels::Masked(colors, alpha) => {
                    let value = if self.bits == 16 {
                        u16::from_le_bytes(source[x * 2..x * 2 + 2].try_into().unwrap()) as u32
                    } else {
                        u32::from_le_bytes(source[x * 4..x * 4 + 4].try_into().unwrap())
                    };
                    rgba.copy_from_slice(&[
                        colors[0].extract(value),
                        colors[1].extract(value),
                        colors[2].extract(value),
                        alpha.map_or(255, |channel| channel.extract(value)),
                    ]);
                }
            }
        }
        Ok(())
    }
}

struct CappedOutput {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for CappedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("encoded clipboard PNG exceeds 64 MiB"));
        }
        let needed = self.bytes.len() + bytes.len();
        if needed > self.bytes.capacity() {
            let capacity = needed.max(
                self.bytes
                    .capacity()
                    .saturating_mul(2)
                    .max(8192)
                    .min(self.limit),
            );
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn dib_to_png(bytes: &[u8], require_v5: bool) -> Result<Vec<u8>> {
    let dib = Dib::parse(bytes, require_v5)?;
    let mut output = CappedOutput {
        bytes: Vec::new(),
        limit: LIMIT,
    };
    let mut encoder = png::Encoder::new(&mut output, dib.width as u32, dib.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fastest);
    let mut writer = encoder.write_header()?;
    let mut row = Vec::new();
    row.try_reserve_exact(dib.width * 4)?;
    row.resize(dib.width * 4, 0);
    {
        let mut stream = writer.stream_writer_with_size(8192)?;
        for y in 0..dib.height {
            dib.row(y, &mut row)?;
            stream.write_all(&row)?;
        }
        stream.finish()?;
    }
    writer.finish()?;
    Ok(output.bytes)
}

#[cfg(test)]
mod tests;
