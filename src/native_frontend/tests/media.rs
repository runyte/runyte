// SPDX-License-Identifier: MPL-2.0
use super::*;

fn key(path: &Path, page: usize) -> Key {
    Key {
        path: path.to_owned(),
        page,
        modified: None,
        length: 0,
    }
}

#[test]
fn image_decode_preserves_alpha_and_converts_rgba_to_native_bgra() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("pixel.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 80, 120]))
        .save(&path)
        .unwrap();
    let (image, pages) = load(&key(&path, 1), &AtomicBool::new(false)).unwrap();
    assert_eq!(pages, 1);
    assert_eq!(&image.image.as_bytes(0).unwrap()[..4], &[80, 40, 20, 120]);
    assert_eq!(image.image.as_bytes(0).unwrap().len(), 16);
}

#[test]
fn corrupt_and_oversized_media_fail_without_publishing_pixels() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("broken.png");
    std::fs::write(&path, b"not an image").unwrap();
    assert!(load(&key(&path, 1), &AtomicBool::new(false)).is_err());
    std::fs::File::create(&path)
        .unwrap()
        .set_len(129 * 1024 * 1024)
        .unwrap();
    assert!(
        load(&key(&path, 1), &AtomicBool::new(false))
            .err()
            .unwrap()
            .to_string()
            .contains("128 MiB")
    );
}

#[test]
#[ignore = "requires installed Poppler pdfinfo and pdftoppm"]
fn pdf_rasterizes_distinct_pages_and_reports_total_count() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("two-pages.pdf");
    std::fs::write(&path, include_bytes!("fixtures/two_pages.pdf")).unwrap();
    let (first, pages) = load(&key(&path, 1), &AtomicBool::new(false)).unwrap();
    let (second, _) = load(&key(&path, 2), &AtomicBool::new(false)).unwrap();
    assert_eq!(pages, 2);
    assert_ne!(first.image.as_bytes(0), second.image.as_bytes(0));
    assert!(load(&key(&path, 3), &AtomicBool::new(false)).is_err());
}

#[cfg(unix)]
#[test]
fn cancelling_pdf_work_kills_and_reaps_the_child() {
    let started = Instant::now();
    assert!(
        wait(
            Command::new("sleep").arg("30"),
            &AtomicBool::new(true),
            None
        )
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn pdf_word_coordinates_are_normalized_and_preserve_reading_lines() {
    let words = parse_words(r#"<html><page width="200" height="100"><line><word xMin="20" yMin="10" xMax="60" yMax="20">Hello &amp;</word><word xMin="70" yMin="10" xMax="100" yMax="20">world</word></line><line><word xMin="20" yMin="30" xMax="60" yMax="40">Next</word></line></page></html>"#).unwrap();
    assert_eq!(words.len(), 3);
    assert_eq!(words[0].bounds, [0.1, 0.1, 0.3, 0.2]);
    assert_eq!(words[0].text, "Hello &");
    assert_eq!(words[2].line, 1);
    assert!(parse_words(r#"<page width="NaN" height="100"/>"#).is_err());
}

#[test]
fn region_copy_exports_selected_rgba_pixels_and_text_copy_keeps_line_breaks() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("region.png");
    image::RgbaImage::from_fn(4, 4, |x, y| {
        image::Rgba([x as u8 * 20, y as u8 * 30, 80, 120])
    })
    .save(&path)
    .unwrap();
    let (mut page, _) = load(&key(&path, 1), &AtomicBool::new(false)).unwrap();
    let cropped = super::super::interactions::crop_rgba(&page, [0.25, 0.25], [0.75, 0.75]).unwrap();
    assert_eq!(cropped.dimensions(), (2, 2));
    assert_eq!(cropped.get_pixel(0, 0).0, [20, 30, 80, 120]);
    assert!(super::super::interactions::crop_rgba(&page, [0., 0.], [0., 0.]).is_err());
    Arc::get_mut(&mut page).unwrap().words = vec![
        Word {
            bounds: [0., 0., 0.2, 0.1],
            text: "Hello".into(),
            line: 0,
        },
        Word {
            bounds: [0.2, 0., 0.4, 0.1],
            text: "world".into(),
            line: 0,
        },
        Word {
            bounds: [0., 0.2, 0.2, 0.3],
            text: "Next".into(),
            line: 1,
        },
    ];
    let mut view = super::super::viewport::Viewport::new(1);
    view.selection = Some(super::super::viewport::Selection::Text(2, 0));
    assert_eq!(
        view.selected_text(&page).as_deref(),
        Some("Hello world\nNext")
    );
    assert_eq!(view.selected_bounds(&page).len(), 3);
    view.selection = None;
    view.begin_selection(&page);
    view.extend_selection(&page, 1, 0);
    assert_eq!(view.selected_text(&page).as_deref(), Some("Hello world"));
    view.extend_selection(&page, 0, 1);
    assert_eq!(
        view.selected_text(&page).as_deref(),
        Some("Hello world\nNext")
    );
}

#[test]
fn source_revision_change_invalidates_pixel_and_text_selections() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.png");
    image::RgbaImage::new(2, 2).save(&path).unwrap();
    let (mut page, _) = load(&key(&path, 1), &AtomicBool::new(false)).unwrap();
    let mut view = super::super::viewport::Viewport::new(1);
    view.show_source(&page);
    view.selection = Some(super::super::viewport::Selection::Region(
        [0., 0.],
        [1., 1.],
    ));
    view.zoom = 3.;
    Arc::get_mut(&mut page).unwrap().source.1 += 1;
    view.show_source(&page);
    assert!(view.selection.is_none());
    assert_eq!(view.zoom, 1.);
}

#[test]
fn jpeg_exif_orientation_is_applied_before_display_and_copy() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rotated.jpg");
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(2, 3)
        .write_to(&mut encoded, image::ImageFormat::Jpeg)
        .unwrap();
    let encoded = encoded.into_inner();
    let exif: &[u8] = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut file = vec![0xff, 0xd8, 0xff, 0xe1];
    file.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    file.extend_from_slice(exif);
    file.extend_from_slice(&encoded[2..]);
    std::fs::write(&path, file).unwrap();
    let (page, _) = load(&key(&path, 1), &AtomicBool::new(false)).unwrap();
    assert_eq!((page.width, page.height), (3., 2.));
}

#[cfg(unix)]
#[test]
fn pdf_helper_output_limit_stops_oversized_output() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("output");
    let error = wait(
        Command::new("head")
            .args(["-c", "1048576", "/dev/zero"])
            .stdout(std::fs::File::create(&path).unwrap()),
        &AtomicBool::new(false),
        Some((&path, 1024)),
    )
    .unwrap_err();
    assert!(error.to_string().contains("size limit"));
}
