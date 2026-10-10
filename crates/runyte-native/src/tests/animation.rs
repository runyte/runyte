// SPDX-License-Identifier: MPL-2.0
use super::*;
use image::{Delay, Frame, Rgba, RgbaImage};

fn gif(repeat: Option<image::codecs::gif::Repeat>, count: usize) -> Vec<u8> {
    let mut data = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut data);
        if let Some(repeat) = repeat {
            encoder.set_repeat(repeat).unwrap();
        }
        for i in 0..count {
            let color = if i % 2 == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            encoder
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(2, 2, Rgba(color)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(if i == 0 { 10 } else { 250 }, 1),
                ))
                .unwrap();
        }
    }
    data
}

fn chunk(tag: &[u8; 4], data: &[u8], target: &mut Vec<u8>) {
    target.extend_from_slice(tag);
    target.extend_from_slice(&(data.len() as u32).to_le_bytes());
    target.extend_from_slice(data);
    if data.len() % 2 == 1 {
        target.push(0);
    }
}

fn webp(plays: u16) -> Vec<u8> {
    let mut data = b"WEBP".to_vec();
    chunk(b"VP8X", &[2, 0, 0, 0, 1, 0, 0, 1, 0, 0], &mut data);
    let mut anim = vec![0, 0, 0, 0];
    anim.extend_from_slice(&plays.to_le_bytes());
    chunk(b"ANIM", &anim, &mut data);
    for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
        let mut still = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut still)
            .encode(
                RgbaImage::from_pixel(2, 2, Rgba(color)).as_raw(),
                2,
                2,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let mut frame = vec![0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 200, 0, 0, 2];
        frame.extend_from_slice(&still[12..]);
        chunk(b"ANMF", &frame, &mut data);
    }
    let mut riff = b"RIFF".to_vec();
    riff.extend_from_slice(&(data.len() as u32).to_le_bytes());
    riff.extend(data);
    riff
}

fn decoded(bytes: &[u8], format: image::ImageFormat) -> Animation {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("animation");
    std::fs::write(&path, bytes).unwrap();
    decode(&path, format, &AtomicBool::new(false))
        .unwrap()
        .unwrap()
}

#[test]
fn gif_frames_normalize_delays_and_distinguish_absent_finite_and_infinite_loops() {
    use image::codecs::gif::Repeat;
    for (repeat, plays) in [
        (None, Some(1)),
        (Some(Repeat::Finite(2)), Some(3)),
        (Some(Repeat::Infinite), None),
    ] {
        let animation = decoded(&gif(repeat, 2), image::ImageFormat::Gif);
        assert_eq!(animation.plays, plays);
        assert_eq!(
            animation.delays,
            [Duration::from_millis(100), Duration::from_millis(250)]
        );
        assert_eq!(
            &animation.frames[0].as_bytes(0).unwrap()[..4],
            &[0, 0, 255, 255]
        );
        assert_eq!(
            &animation.frames[1].as_bytes(0).unwrap()[..4],
            &[255, 0, 0, 255]
        );
        assert_eq!(animation.bytes(), 32);
    }
}

#[test]
fn webp_frames_use_file_delays_and_total_loop_count() {
    for (count, plays) in [(0, None), (1, Some(1)), (3, Some(3))] {
        let animation = decoded(&webp(count), image::ImageFormat::WebP);
        assert_eq!(animation.plays, plays);
        assert_eq!(animation.delays, [Duration::from_millis(200); 2]);
        assert_eq!(
            &animation.frames[1].as_bytes(0).unwrap()[..4],
            &[255, 0, 0, 255]
        );
    }
}

#[test]
fn gif_partial_frames_are_composited_with_background_disposal() {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        // The encoder uses Background disposal: frame two must not inherit
        // red pixels outside its smaller rectangle from frame one.
        encoder
            .encode_frame(Frame::new(RgbaImage::from_pixel(
                2,
                2,
                Rgba([255, 0, 0, 255]),
            )))
            .unwrap();
        encoder
            .encode_frame(Frame::new(RgbaImage::from_pixel(
                1,
                1,
                Rgba([0, 0, 255, 255]),
            )))
            .unwrap();
    }
    let animation = decoded(&bytes, image::ImageFormat::Gif);
    assert_eq!(animation.frames[0].size(0), animation.frames[1].size(0));
    let second = animation.frames[1].as_bytes(0).unwrap();
    assert_eq!(&second[..4], &[255, 0, 0, 255]);
    assert_eq!(&second[4..], &[0; 12]);
}

#[test]
fn animation_frames_are_reduced_before_the_retained_budget_is_charged() {
    let frames = image::Frames::new(Box::new(std::iter::once(Ok(Frame::new(
        RgbaImage::from_pixel(4096, 1, Rgba([3, 7, 11, 127])),
    )))));
    let animation = collect(
        frames,
        Some(1),
        image::metadata::Orientation::NoTransforms,
        &AtomicBool::new(false),
        2048 * 4,
        1,
    )
    .unwrap();
    assert_eq!(animation.frames[0].size(0).width.0, 2048);
    assert_eq!(animation.bytes(), 2048 * 4);
    assert_eq!(
        &animation.frames[0].as_bytes(0).unwrap()[..4],
        &[11, 7, 3, 127]
    );
    assert!(
        Playback::default()
            .update(&animation, Instant::now(), false)
            .is_none()
    );
}

#[test]
fn still_gifs_keep_the_original_decoder_and_cancelled_sources_stop() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("single.gif");
    std::fs::write(&path, gif(None, 1)).unwrap();
    assert!(
        decode(&path, image::ImageFormat::Gif, &AtomicBool::new(false))
            .unwrap()
            .is_none()
    );
    assert!(decode(&path, image::ImageFormat::Gif, &AtomicBool::new(true)).is_err());
    std::fs::write(&path, b"GIF89a").unwrap();
    assert!(decode(&path, image::ImageFormat::Gif, &AtomicBool::new(false)).is_err());
}

#[test]
fn frame_memory_count_and_canvas_limits_fail_before_unbounded_collection() {
    let frames = || {
        image::Frames::new(Box::new(
            (0..3).map(|_| Ok(Frame::new(RgbaImage::new(2, 2)))),
        ))
    };
    let orientation = image::metadata::Orientation::NoTransforms;
    assert!(collect(frames(), None, orientation, &AtomicBool::new(false), 31, 10).is_err());
    assert!(collect(frames(), None, orientation, &AtomicBool::new(false), 100, 2).is_err());
    assert!(collect(frames(), None, orientation, &AtomicBool::new(true), 100, 10).is_err());
    assert!(check_dimensions((8192, 8192)).is_err());
    assert!(check_dimensions((8193, 1)).is_err());
    assert!(check_dimensions((0, 1)).is_err());
    assert!(check_dimensions((4096, 4096)).is_ok());
}

#[test]
fn playback_finishes_last_frame_and_paused_split_has_its_own_clock() {
    let animation = decoded(&gif(None, 2), image::ImageFormat::Gif);
    let mut playing = Playback::default();
    let mut selected = Playback::default();
    let now = Instant::now();
    assert_eq!(
        playing.update(&animation, now, false),
        Some(now + Duration::from_millis(100))
    );
    assert_eq!(selected.update(&animation, now, true), None);
    assert_eq!(
        playing.update(&animation, now + Duration::from_millis(100), false),
        Some(now + Duration::from_millis(350))
    );
    assert_eq!(playing.frame, 1);
    assert_eq!(selected.frame, 0);
    assert_eq!(
        playing.update(&animation, now + Duration::from_secs(1), false),
        None
    );
    assert_eq!(playing.frame, 1);
    assert_eq!(
        selected.update(&animation, now + Duration::from_secs(10), false),
        Some(now + Duration::from_millis(10100))
    );
}

#[test]
fn playback_suspends_hidden_clocks_without_catching_up() {
    let animation = decoded(
        &gif(Some(image::codecs::gif::Repeat::Infinite), 2),
        image::ImageFormat::Gif,
    );
    let mut playback = Playback::default();
    let now = Instant::now();
    playback.update(&animation, now, false);
    playback.suspend();
    playback.update(&animation, now + Duration::from_secs(100), false);
    assert_eq!(playback.frame, 0);
    playback.update(&animation, now + Duration::from_secs(200), false);
    assert_eq!(
        playback.frame, 1,
        "a long unpresented interval advances only once"
    );
    playback.update(&animation, now + Duration::from_secs(201), false);
    assert_eq!(playback.frame, 0);
}

#[test]
fn playback_counts_full_repetitions_then_remains_finished_after_selection() {
    let animation = decoded(
        &gif(Some(image::codecs::gif::Repeat::Finite(2)), 2),
        image::ImageFormat::Gif,
    );
    let mut playback = Playback::default();
    let now = Instant::now();
    for (millis, frame) in [(0, 0), (100, 1), (350, 0), (450, 1), (700, 0), (800, 1)] {
        assert!(
            playback
                .update(&animation, now + Duration::from_millis(millis), false)
                .is_some()
        );
        assert_eq!(playback.frame, frame);
    }
    assert!(
        playback
            .update(&animation, now + Duration::from_millis(1050), false)
            .is_none()
    );
    assert_eq!(playback.frame, 1);
    playback.update(&animation, now + Duration::from_secs(2), true);
    assert!(
        playback
            .update(&animation, now + Duration::from_secs(3), false)
            .is_none()
    );
}

#[test]
fn fully_covered_or_empty_panes_do_not_animate() {
    use runyte::layout::Rect;
    let body = Rect {
        x: 2,
        y: 2,
        width: 4,
        height: 4,
    };
    assert!(visible(body, &[]));
    assert!(!visible(Rect { width: 0, ..body }, &[]));
    assert!(!visible(body, &[body]));
    assert!(!visible(
        body,
        &[
            Rect { width: 2, ..body },
            Rect {
                x: 4,
                width: 2,
                ..body
            }
        ]
    ));
    assert!(visible(body, &[Rect { width: 2, ..body }]));
}

#[test]
fn webp_metadata_is_bounded_before_allocating_and_source_reads_are_cancellable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("bad.webp");
    let mut bytes = b"RIFF\0\0\0\0WEBPEXIF".to_vec();
    bytes.extend_from_slice(&65537u32.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    let cancel = AtomicBool::new(false);
    assert!(
        webp_orientation(Source::open(&path, &cancel).unwrap())
            .unwrap_err()
            .to_string()
            .contains("64 KiB")
    );
    let mut source = Source::open(&path, &cancel).unwrap();
    cancel.store(true, Ordering::Release);
    assert!(source.read(&mut [0]).is_err());
}
