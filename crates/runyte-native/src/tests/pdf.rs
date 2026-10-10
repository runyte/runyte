// SPDX-License-Identifier: MPL-2.0
use super::*;

pub(crate) fn document(content: &str, page_options: &str, font: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] {page_options} /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        ),
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{font} >>"),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    pdf
}

fn fixture(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fixture.pdf");
    std::fs::write(&path, bytes).unwrap();
    (root, path)
}

#[test]
fn hayro_renders_the_authored_fixture_and_extracts_real_words() {
    let (_root, path) = fixture(include_bytes!("fixtures/two_pages.pdf"));
    let first = render::page(&path, 1, None).unwrap();
    let second = render::page(&path, 2, None).unwrap();
    assert_eq!(first.header.pages, 2);
    assert_ne!(first.pixels, second.pixels);
    assert_eq!(
        first
            .header
            .words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>(),
        ["Hello", "Runyte"]
    );
    assert!(first.header.text_error.is_none());
    assert!(first.pixels.chunks_exact(4).all(|p| p[3] == 255));
}

#[test]
fn hayro_word_geometry_tracks_rotation_and_nonzero_crop_origins() {
    for rotation in [0, 90, 180, 270] {
        let (_root, path) = fixture(&document(
            "BT /F1 20 Tf 50 80 Td (Hello World) Tj ET",
            &format!("/CropBox [20 30 280 180] /Rotate {rotation}"),
            "Helvetica",
        ));
        let raster = render::page(&path, 1, None).unwrap();
        assert_eq!(
            raster
                .header
                .words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>(),
            ["Hello", "World"]
        );
        for word in &raster.header.words {
            assert!(word.bounds.iter().all(|n| (0.0..=1.0).contains(n)));
            let [x0, y0, x1, y1] = word.bounds;
            let mut dark = 0;
            for y in (y0 * raster.header.height as f32).floor() as u32
                ..(y1 * raster.header.height as f32).ceil() as u32
            {
                for x in (x0 * raster.header.width as f32).floor() as u32
                    ..(x1 * raster.header.width as f32).ceil() as u32
                {
                    if raster.pixels[((y * raster.header.width + x) * 4) as usize] < 100 {
                        dark += 1;
                    }
                }
            }
            assert!(
                dark > 50,
                "word bounds missed rendered text at rotation {rotation}"
            );
        }
    }
}

#[test]
fn hayro_detail_is_a_crop_of_the_same_scaled_page() {
    let (_root, path) = fixture(&document(
        "0.2 0.4 0.8 rg 40 60 80 70 re f BT /F1 18 Tf 50 80 Td (Detail) Tj ET",
        "/CropBox [20 30 280 180] /Rotate 90",
        "Helvetica",
    ));
    let full = render::page(
        &path,
        1,
        Some(Detail {
            full: [1200, 1600],
            origin: [0, 0],
            size: [1200, 1600],
        }),
    )
    .unwrap();
    let detail = Detail {
        full: [1200, 1600],
        origin: [300, 400],
        size: [500, 600],
    };
    let cropped = render::page(&path, 1, Some(detail)).unwrap();
    let expected: Vec<_> = (400..1000)
        .flat_map(|y| {
            full.pixels[(y * 1200 + 300) * 4..(y * 1200 + 800) * 4]
                .iter()
                .copied()
        })
        .collect();
    assert_eq!(cropped.pixels, expected);
    assert!(cropped.header.words.is_empty());
    let huge = render::page(
        &path,
        1,
        Some(Detail {
            full: [524288, 524288],
            origin: [100, 100],
            size: [32, 32],
        }),
    )
    .unwrap();
    assert_eq!(huge.pixels.len(), 32 * 32 * 4);
}

#[test]
fn text_collection_keeps_ocr_and_deduplicates_fill_stroke_and_column_boundaries() {
    let content = "BT /F1 12 Tf 20 170 Td 2 Tr (Left One) Tj 0 -20 Td 3 Tr (Left Two) Tj ET BT /F1 12 Tf 180 170 Td 0 Tr (Right One) Tj 0 -20 Td (Right Two) Tj ET";
    let (_root, path) = fixture(&document(content, "", "Helvetica"));
    let raster = render::page(&path, 1, None).unwrap();
    assert_eq!(
        raster
            .header
            .words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>(),
        ["Left", "One", "Left", "Two", "Right", "One", "Right", "Two"]
    );
    assert_eq!(
        raster
            .header
            .words
            .iter()
            .map(|w| w.line)
            .collect::<Vec<_>>(),
        [0, 0, 1, 1, 2, 2, 3, 3]
    );
}

#[test]
fn hayro_rejects_missing_fonts_bad_pages_and_damaged_documents() {
    let (_root, path) = fixture(&document(
        "BT /F1 12 Tf 20 50 Td (Missing) Tj ET",
        "",
        "UninstalledTestFont",
    ));
    assert!(
        render::page(&path, 1, None)
            .err()
            .unwrap()
            .to_string()
            .contains("font")
    );
    assert!(render::page(&path, 0, None).is_err());
    assert!(render::page(&path, 2, None).is_err());
    std::fs::write(&path, b"not a PDF").unwrap();
    assert!(render::page(&path, 1, None).is_err());
}

#[test]
fn private_pdf_response_rejects_truncation_overlarge_headers_and_bad_geometry() {
    let (_root, path) = fixture(&document("0 0 1 rg 10 10 30 30 re f", "", "Helvetica"));
    let raster = render::page(&path, 1, None).unwrap();
    let json = serde_json::to_vec(&Ok::<_, String>(&raster.header)).unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(&raster.pixels);
    assert_eq!(decode(&bytes, None).unwrap().pixels, raster.pixels);
    assert!(decode(&bytes[..bytes.len() - 1], None).is_err());
    bytes[8..12].copy_from_slice(&((MAX_HEADER + 1) as u32).to_le_bytes());
    assert!(decode(&bytes, None).is_err());
    assert!(
        validate_detail(Detail {
            full: [100, 100],
            origin: [90, 0],
            size: [20, 20]
        })
        .is_err()
    );
    assert!(
        validate_detail(Detail {
            full: [524288, 524288],
            origin: [0, 0],
            size: [4096, 4096]
        })
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn helper_transport_bounds_output_timeout_cancellation_and_descendant_pipes() {
    let root = tempfile::tempdir().unwrap();
    let command = |script: &str| {
        let mut c = Command::new("sh");
        c.args(["-c", script]).env("XDG_CONFIG_HOME", root.path());
        c
    };
    let cancel = AtomicBool::new(false);
    assert_eq!(
        run(
            &mut command("printf bounded"),
            &cancel,
            Duration::from_secs(1),
            100
        )
        .unwrap(),
        b"bounded"
    );
    assert!(
        run(
            &mut command("exec yes x"),
            &cancel,
            Duration::from_secs(1),
            10
        )
        .unwrap_err()
        .to_string()
        .contains("size limit")
    );
    let start = Instant::now();
    assert!(
        run(
            &mut command("sleep 10 & exit 0"),
            &cancel,
            Duration::from_millis(50),
            100
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        signal.store(true, Ordering::Release);
    });
    assert!(
        run(
            &mut command("exec sleep 10"),
            &cancel,
            Duration::from_secs(1),
            100
        )
        .unwrap_err()
        .to_string()
        .contains("cancelled")
    );
    trigger.join().unwrap();
}

#[test]
fn recognized_nonembedded_latin_fonts_use_documented_standard_substitutes() {
    let (_root, path) = fixture(&document(
        "BT /F1 12 Tf 20 50 Td (Substitute) Tj ET",
        "",
        "MyCustomSans",
    ));
    let raster = render::page(&path, 1, None).unwrap();
    assert_eq!(raster.header.words[0].text, "Substitute");
}
