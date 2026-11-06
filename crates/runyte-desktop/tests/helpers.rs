// SPDX-License-Identifier: MPL-2.0
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn invoke(args: &[&str], input: &[u8]) -> Output {
    let config = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_runyte-desktop"))
        .args(args)
        .env("XDG_CONFIG_HOME", config.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
fn request() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "text": include_str!("../../runyte-preview/fixtures/document.html"),
        "language":"html", "path":null, "width":64, "height":48,
        "scale":1, "scroll":[0,0], "selection":null
    }))
    .unwrap()
}
fn frame(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|b| *b == b'\n').unwrap();
    let header: serde_json::Value = serde_json::from_slice(&bytes[..end]).unwrap();
    assert_eq!(header["width"], 64);
    assert_eq!(header["height"], 48);
    let pixels =
        header["raster_width"].as_u64().unwrap() * header["raster_height"].as_u64().unwrap() * 4;
    &bytes[end + 1 + pixels as usize..]
}
#[test]
fn preview_one_shot_and_retained_frames_use_the_desktop_executable() {
    let request = request();
    let once = invoke(&["--helper", "preview"], &request);
    assert!(
        once.status.success(),
        "{}",
        String::from_utf8_lossy(&once.stderr)
    );
    assert!(frame(&once.stdout).is_empty());
    let mut requests = request.clone();
    requests.push(b'\n');
    requests.extend(request);
    requests.push(b'\n');
    let retained = invoke(&["--helper", "preview", "--serve"], &requests);
    assert!(
        retained.status.success(),
        "{}",
        String::from_utf8_lossy(&retained.stderr)
    );
    assert!(frame(frame(&retained.stdout)).is_empty());
}

#[test]
fn preview_helpers_admit_one_mib_after_worst_case_json_expansion() {
    let start = "<!--";
    let end = "--><body>Large capture</body>";
    let text = format!(
        "{start}{}{end}",
        "\u{1}".repeat(runyte::document_preview::MAX_BYTES - start.len() - end.len())
    );
    let mut request: serde_json::Value = serde_json::from_slice(&request()).unwrap();
    request["text"] = text.into();
    let bytes = serde_json::to_vec(&request).unwrap();
    assert!(bytes.len() > 6 * 1024 * 1024 - 1024);
    for retained in [false, true] {
        let mut input = bytes.clone();
        let args = if retained {
            input.push(b'\n');
            &["--helper", "preview", "--serve"][..]
        } else {
            &["--helper", "preview"][..]
        };
        let output = invoke(args, &input);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(frame(&output.stdout).is_empty());
    }
    request["text"] = format!("{}x", request["text"].as_str().unwrap()).into();
    let bytes = serde_json::to_vec(&request).unwrap();
    let output = invoke(&["--helper", "preview"], &bytes);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("input or viewport budget"));
}

#[test]
fn preview_helper_copies_a_rendered_selection_above_the_old_limit() {
    let text = format!("{}END", format!("{}\n", "\"".repeat(1000)).repeat(150));
    let request = serde_json::to_vec(&serde_json::json!({
        "text": format!("<body style='margin:0'><pre style='margin:0;font:1px/1px monospace'>{text}</pre></body>"),
        "language": "html", "path": null, "width": 1000, "height": 200,
        "scale": 1, "scroll": [0, 0], "selection": [[0, 0.5], [1.5, 150.5]]
    })).unwrap();
    let output = invoke(&["--helper", "preview"], &request);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let end = output
        .stdout
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap();
    assert!(end > 200_000);
    let header: serde_json::Value = serde_json::from_slice(&output.stdout[..end]).unwrap();
    let selected = header["selected"].as_str().unwrap();
    assert!(selected.len() > 128 * 1024);
    assert_eq!(
        selected.bytes().filter(|byte| *byte == b'"').count(),
        150_000
    );
}
/// The first pixel of a Markdown page rendered with `theme`, as RGBA.
fn markdown_ground(theme: serde_json::Value) -> [u8; 4] {
    let request = serde_json::to_vec(&serde_json::json!({
        "text": "# Title\n\nBody text.\n",
        "language": "markdown", "path": null, "width": 64, "height": 48,
        "scale": 1, "scroll": [0, 0], "selection": null, "theme": theme
    }))
    .unwrap();
    let output = invoke(&["--helper", "preview"], &request);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(frame(&output.stdout).is_empty());
    let end = output.stdout.iter().position(|b| *b == b'\n').unwrap();
    output.stdout[end + 1..end + 5].try_into().unwrap()
}
#[test]
fn preview_pages_take_the_editor_theme_ground_or_stay_white() {
    let config = runyte::config::Config::default();
    assert_eq!(markdown_ground(serde_json::Value::Null), [0xff; 4]);
    for name in config.theme_names() {
        let theme = config.resolve_theme(name).unwrap();
        let palette = runyte::document_preview::Palette::from_theme(&theme);
        let ground = markdown_ground(serde_json::to_value(&palette).unwrap());
        for channel in 0..3 {
            assert!(
                ground[channel].abs_diff(palette.background[channel]) <= 1,
                "{name}: {ground:?} against {:?}",
                palette.background
            );
        }
    }
}
#[test]
fn pdf_helper_returns_a_real_page_before_editor_startup() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runyte-native/src/tests/fixtures/two_pages.pdf");
    let output = invoke(&["--helper", "pdf", path.to_str().unwrap(), "1"], b"");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(&output.stdout[..8], b"RYTPDF01");
    let size = u32::from_le_bytes(output.stdout[8..12].try_into().unwrap()) as usize;
    let header: serde_json::Value = serde_json::from_slice(&output.stdout[12..12 + size]).unwrap();
    assert_eq!(header["Ok"]["pages"], 2);
    assert!(output.stdout.len() > 12 + size);
}
#[test]
fn invalid_helper_roles_and_arguments_fail_without_opening_the_editor() {
    for args in [
        &["--helper"][..],
        &["--helper", "unknown"],
        &["--helper", "preview", "unknown"],
        &["--helper", "preview", "--serve", "extra"],
    ] {
        assert!(!invoke(args, b"").status.success(), "{args:?}");
    }
    assert!(!String::from_utf8_lossy(&invoke(&["--help"], b"").stdout).contains("--helper"));
}

#[test]
fn desktop_help_and_version_identify_the_edition() {
    let version = invoke(&["--version"], b"");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("runyte {} (desktop edition)\n", runyte::VERSION)
    );
    let help = invoke(&["--help"], b"");
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--window"));
    assert!(help.ends_with("Desktop edition.\n"));
}
