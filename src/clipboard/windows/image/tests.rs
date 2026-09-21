// SPDX-License-Identifier: MPL-2.0

use super::*;

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn bitmap(width: i32, height: i32, bits: u16, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 40];
    put_u32(&mut bytes, 0, 40);
    put_u32(&mut bytes, 4, width as u32);
    put_u32(&mut bytes, 8, height as u32);
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&bits.to_le_bytes());
    put_u32(&mut bytes, 20, pixels.len() as u32);
    bytes.extend_from_slice(pixels);
    bytes
}

fn extended(bytes: &mut Vec<u8>, size: usize) {
    bytes.splice(40..40, std::iter::repeat_n(0, size - 40));
    put_u32(bytes, 0, size as u32);
    put_u32(bytes, 56, SRGB);
}

fn pixels(bytes: &[u8], require_v5: bool) -> (u32, u32, Vec<u8>) {
    let encoded = dib_to_png(bytes, require_v5).unwrap();
    let mut decoder = png::Decoder::new(std::io::Cursor::new(encoded))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
    let info = decoder.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

#[test]
fn padded_bgr_rows_preserve_colors_and_both_orientations() {
    let mut bytes = bitmap(1, 2, 24, &[255, 0, 0, 99, 0, 0, 255, 88]);
    assert_eq!(
        pixels(&bytes, false),
        (1, 2, vec![255, 0, 0, 255, 0, 0, 255, 255])
    );
    put_u32(&mut bytes, 8, (-2i32) as u32);
    assert_eq!(pixels(&bytes, false).2, [0, 0, 255, 255, 255, 0, 0, 255]);
    // A BI_RGB bitmap may omit its image-size field.
    put_u32(&mut bytes, 20, 0);
    assert_eq!(pixels(&bytes, false).2, [0, 0, 255, 255, 255, 0, 0, 255]);
}

#[test]
fn palettes_decode_one_four_and_eight_bit_indices() {
    for (bits, packed) in [
        (1, [0xa0, 0, 0, 0]),
        (4, [0x10, 0x10, 0, 0]),
        (8, [1, 0, 1, 0]),
    ] {
        let mut bytes = bitmap(3, 1, bits, &packed);
        put_u32(&mut bytes, 32, 2);
        bytes.splice(40..40, [255, 0, 0, 0, 0, 0, 255, 0]);
        assert_eq!(
            pixels(&bytes, false).2,
            [255, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255]
        );
        if bits == 1 {
            put_u32(&mut bytes, 32, 0); // Default palette has 2^bpp entries.
            assert_eq!(pixels(&bytes, false).2[0..4], [255, 0, 0, 255]);
        }
    }
}

#[test]
fn bgrx_is_opaque_and_only_an_explicit_mask_supplies_alpha() {
    let mut bytes = bitmap(2, 1, 32, &[30, 20, 10, 0, 60, 50, 40, 128]);
    assert_eq!(pixels(&bytes, false).2, [10, 20, 30, 255, 40, 50, 60, 255]);
    extended(&mut bytes, 124);
    assert_eq!(pixels(&bytes, true).2[3], 255);
    put_u32(&mut bytes, 52, 0xff00_0000);
    assert_eq!(pixels(&bytes, true).2, [10, 20, 30, 0, 40, 50, 60, 128]);
    put_u32(&mut bytes, 16, BITFIELDS);
    for (offset, mask) in [(40, 0x00ff_0000), (44, 0x0000_ff00), (48, 0x0000_00ff)] {
        put_u32(&mut bytes, offset, mask);
    }
    assert_eq!(pixels(&bytes, true).2, [10, 20, 30, 0, 40, 50, 60, 128]);
}

#[test]
fn rgb555_and_explicit_rgb565_and_four_bit_channels_are_scaled() {
    let bytes = bitmap(1, 1, 16, &[0, 0x7c, 0, 0]);
    assert_eq!(pixels(&bytes, false).2, [255, 0, 0, 255]);
    let mut bytes = bitmap(1, 1, 16, &[0xe0, 0x07, 0, 0]);
    put_u32(&mut bytes, 16, BITFIELDS);
    bytes.splice(
        40..40,
        [0xf800u32, 0x07e0, 0x001f]
            .into_iter()
            .flat_map(u32::to_le_bytes),
    );
    assert_eq!(pixels(&bytes, false).2, [0, 255, 0, 255]);
    let mut bytes = bitmap(1, 1, 32, &0x8123u32.to_le_bytes());
    extended(&mut bytes, 108);
    put_u32(&mut bytes, 16, BITFIELDS);
    for (offset, mask) in [(40, 0x0f00), (44, 0x00f0), (48, 0x000f), (52, 0xf000)] {
        put_u32(&mut bytes, offset, mask);
    }
    assert_eq!(pixels(&bytes, false).2, [17, 34, 51, 136]);
}

#[test]
fn optimal_truecolor_palettes_are_skipped_before_reading_pixels() {
    let mut bytes = bitmap(1, 1, 24, &[3, 2, 1, 0]);
    put_u32(&mut bytes, 32, 1);
    bytes.splice(40..40, [90, 80, 70, 0]);
    assert_eq!(pixels(&bytes, false).2, [1, 2, 3, 255]);
}

#[test]
fn malformed_headers_masks_profiles_and_dimensions_fail_before_encoding() {
    let base = bitmap(1, 1, 32, &[3, 2, 1, 0]);
    for (offset, value) in [
        (0, 12),
        (0, 56),
        (0, 124),
        (4, 0),
        (4, u32::MAX),
        (8, 0),
        (8, i32::MIN as u32),
        (4, i32::MAX as u32),
        (8, i32::MAX as u32),
        (16, 1),
        (16, 4),
        (16, 5),
        (20, 3),
        (20, 999),
        (32, u32::MAX),
    ] {
        let mut bytes = base.clone();
        put_u32(&mut bytes, offset, value);
        assert!(
            Dib::parse(&bytes, false).is_err(),
            "offset {offset}, value {value}"
        );
    }
    assert!(Dib::parse(&base, true).is_err());
    for (offset, value) in [(12, 2u16), (14, 2), (14, 64)] {
        let mut bytes = base.clone();
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        assert!(Dib::parse(&bytes, false).is_err());
    }
    let mut bytes = base.clone();
    extended(&mut bytes, 124);
    for (offset, value) in [
        (56, 0),
        (56, 0x4c49_4e4b),
        (56, 0x4d42_4544),
        (112, 124),
        (116, 1),
        (120, 1),
    ] {
        let mut invalid = bytes.clone();
        put_u32(&mut invalid, offset, value);
        assert!(Dib::parse(&invalid, true).is_err());
    }
    put_u32(&mut bytes, 16, BITFIELDS);
    for (offset, mask) in [(40, 0x00ff_0000), (44, 0x0000_ff00), (48, 0x0000_00ff)] {
        put_u32(&mut bytes, offset, mask);
    }
    for (offset, value) in [(40, 0), (40, 0x0055_0000), (40, 0x0000_ff00), (52, 0xff)] {
        let mut invalid = bytes.clone();
        put_u32(&mut invalid, offset, value);
        assert!(Dib::parse(&invalid, true).is_err());
    }
    for cut in 0..base.len() {
        assert!(Dib::parse(&base[..cut], false).is_err(), "length {cut}");
    }
}

#[test]
fn invalid_palette_indices_and_mask_widths_are_refused() {
    let mut bytes = bitmap(1, 1, 8, &[2, 0, 0, 0]);
    put_u32(&mut bytes, 32, 2);
    bytes.splice(40..40, [0; 8]);
    assert!(
        dib_to_png(&bytes, false)
            .unwrap_err()
            .to_string()
            .contains("index")
    );
    put_u32(&mut bytes, 32, 257);
    assert!(Dib::parse(&bytes, false).is_err());
    let mut bytes = bitmap(1, 1, 16, &[0; 4]);
    put_u32(&mut bytes, 16, BITFIELDS);
    bytes.splice(
        40..40,
        [0x10_0000u32, 0x07e0, 0x001f]
            .into_iter()
            .flat_map(u32::to_le_bytes),
    );
    assert!(Dib::parse(&bytes, false).is_err());
    let mut bytes = bitmap(1, 1, 24, &[0; 4]);
    put_u32(&mut bytes, 16, BITFIELDS);
    assert!(Dib::parse(&bytes, false).is_err());
}

#[test]
fn decoded_dimensions_and_encoded_bytes_have_independent_limits() {
    // A tiny header must not allocate the bitmap it advertises.
    let bytes = bitmap((LIMIT / 4 + 1) as i32, 1, 1, &[]);
    assert!(
        Dib::parse(&bytes, false)
            .err()
            .unwrap()
            .to_string()
            .contains("decoded")
    );
    let mut sink = CappedOutput {
        bytes: Vec::new(),
        limit: 4,
    };
    sink.write_all(b"1234").unwrap();
    assert!(sink.write_all(b"5").is_err());
    assert_eq!(sink.bytes, b"1234");
    let mut sink = CappedOutput {
        bytes: Vec::new(),
        limit: 32,
    };
    let mut encoder = png::Encoder::new(&mut sink, 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    assert!(
        encoder.write_header().is_err(),
        "PNG framing bypassed output limit"
    );
}
