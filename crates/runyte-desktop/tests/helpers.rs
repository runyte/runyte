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
