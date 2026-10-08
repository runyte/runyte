// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;

#[test]
fn native_media_opens_through_commands_and_explorer_without_a_save_target() {
    let root = TestRuntimeRoot::new("media-open").unwrap();
    let path = root.join("photo.png");
    let original = b"\x89PNG\0binary fixture";
    fs::write(&path, original).unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.execute(parse_colon_command(&format!("open {}", path.display())).unwrap())
        .unwrap();
    assert_eq!(
        app.active_buffer().media_path.as_deref(),
        Some(path.as_path())
    );
    assert!(app.active_buffer().path.is_none());
    assert!(app.active_buffer().is_read_only());
    let id = app.active().buffer;
    let text = app.active_buffer().text().to_string();
    press(&mut app, 'd');
    assert_eq!(app.active_buffer().text().to_string(), text);
    app.execute(parse_colon_command("write").unwrap()).unwrap();
    assert!(!app.active_buffer().dirty);
    assert_eq!(fs::read(&path).unwrap(), original);

    app.open_file(root.to_path_buf()).unwrap();
    let row = (0..app.active_buffer().text().len_lines())
        .find(|row| {
            app.active_buffer()
                .text()
                .line_string(*row)
                .contains("photo.png")
        })
        .unwrap();
    let offset = app.active_buffer().text().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, id, "explorer reuses the media buffer");
    assert!(app.external_target.is_none());
}

#[test]
fn native_launch_keeps_text_positions_and_binary_fallback() {
    let root = TestRuntimeRoot::new("media-launch").unwrap();
    let path = root.join("notes.txt");
    fs::write(&path, "first\nsecond\nthird").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_native_target(LaunchTarget::at(
        &path,
        LaunchPosition {
            line: std::num::NonZeroUsize::new(2).unwrap(),
            column: None,
        },
    ))
    .unwrap();
    assert_eq!(app.active().cursor(app.active_buffer()).row, 1);
    assert!(!app.active_buffer().is_read_only());
    let binary = root.join("unknown.bin");
    fs::write(&binary, b"\0binary").unwrap();
    app.open_file(binary).unwrap();
    assert!(app.external_target.is_some());
}

#[test]
fn terminal_frontend_keeps_external_media_opening() {
    let root = TestRuntimeRoot::new("media-tui").unwrap();
    let path = root.join("photo.png");
    fs::write(&path, b"\0PNG").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.open_file(path).unwrap();
    assert!(app.external_target.is_some());
    assert!(app.buffers.iter().all(|buffer| buffer.media_path.is_none()));
}

#[test]
fn missing_media_does_not_replace_the_current_pane() {
    let root = TestRuntimeRoot::new("media-missing").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    let id = app.active().buffer;
    assert!(app.open_file(root.join("missing.PDF")).is_err());
    assert_eq!(app.active().buffer, id);
}

#[test]
fn native_startup_preserves_first_target_and_first_explicit_position() {
    let root = TestRuntimeRoot::new("media-targets").unwrap();
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    fs::write(&first, "one\ntwo\nthree").unwrap();
    fs::write(&second, "other").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    let at = |line| {
        LaunchTarget::at(
            &first,
            LaunchPosition {
                line: std::num::NonZeroUsize::new(line).unwrap(),
                column: None,
            },
        )
    };
    app.open_native_targets(vec![
        LaunchTarget::new(&first),
        at(2),
        LaunchTarget::new(second),
        at(3),
    ])
    .unwrap();
    assert_eq!(app.active_buffer().path.as_ref(), Some(&first));
    assert_eq!(app.active().cursor(app.active_buffer()).row, 1);
}

#[cfg(unix)]
#[test]
fn native_media_refuses_fifo_without_reading_it() {
    let root = TestRuntimeRoot::new("media-fifo").unwrap();
    let path = root.join("pipe.png");
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: the temporary path is NUL terminated and lives for the call.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    assert!(
        app.open_file(path)
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
}

#[test]
fn native_media_bindings_use_the_registry_and_preserve_pdf_page_motions() {
    use crate::{keymap::BindingScope, media::ViewAction};
    let root = TestRuntimeRoot::new("media-keys").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    assert_eq!(app.key_binding_scope(), BindingScope::Media);
    let id = app.active().buffer;
    app.buffers[id].replace_virtual_text("Page 1\nPage 2\nPage 3");
    press(&mut app, 'j');
    assert_eq!(
        app.active_buffer()
            .position_of(app.active().selection.primary().head)
            .row,
        1
    );
    key(&mut app, KeyCode::Char('f'), Modifiers::CONTROL);
    assert_eq!(
        app.active_buffer()
            .position_of(app.active().selection.primary().head)
            .row,
        2
    );
    press(&mut app, '+');
    press(&mut app, 'z');
    press(&mut app, 'f');
    press(&mut app, 'z');
    press(&mut app, 'j');
    press(&mut app, 'y');
    assert_eq!(
        app.media_requests
            .iter()
            .map(|request| request.action)
            .collect::<Vec<_>>(),
        vec![
            ViewAction::ZoomIn,
            ViewAction::Fit,
            ViewAction::PanDown,
            ViewAction::CopySelection
        ]
    );
    assert!(app.media_requests.iter().all(|request| request.path == path
        && request.pane == app.active_pane
        && request.page == 3));
    assert_eq!(fs::read(path).unwrap(), b"%PDF fixture");
}

#[test]
fn native_media_mouse_targeting_does_not_treat_image_pixels_as_pdf_rows() {
    let root = TestRuntimeRoot::new("media-pointer").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    let id = app.active().buffer;
    let pane = app.active_pane;
    app.buffers[id].replace_virtual_text("Page 1\nPage 2\nPage 3");
    app.navigate_native_media(pane, &path, 0);
    assert_eq!(app.active().selection.primary().head, 0);
    app.navigate_native_media(pane, &path, 2);
    assert_eq!(
        app.active_buffer()
            .position_of(app.active().selection.primary().head)
            .row,
        2
    );
    app.navigate_native_media(pane, &root.join("different.pdf"), -2);
    assert_eq!(
        app.active_buffer()
            .position_of(app.active().selection.primary().head)
            .row,
        2
    );
}

#[test]
fn media_select_mode_extends_native_selection_without_changing_pdf_pages() {
    use crate::media::ViewAction;
    let root = TestRuntimeRoot::new("media-select").unwrap();
    let path = root.join("page.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path).unwrap();
    press(&mut app, 'v');
    press(&mut app, 'l');
    press(&mut app, 'j');
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary().head, 0);
    assert_eq!(
        app.media_requests
            .iter()
            .map(|request| request.action)
            .collect::<Vec<_>>(),
        vec![
            ViewAction::BeginSelection,
            ViewAction::ExtendRight,
            ViewAction::ExtendDown
        ]
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.media_requests.back().unwrap().action,
        ViewAction::ClearSelection
    );
}

#[test]
fn pdf_page_picker_filters_accepts_and_cancels_without_changing_source() {
    let root = TestRuntimeRoot::new("media-page-picker").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].replace_virtual_text("Page 1 of 4\nPage 2 of 4\nPage 3 of 4\nPage 4 of 4");
    press(&mut app, 'j');
    press(&mut app, 'g');
    press(&mut app, 'p');
    assert_eq!(app.list.as_ref().unwrap().selected, 1);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.list.is_none());
    assert_eq!(app.active().cursor(app.active_buffer()).row, 2);
    for ch in ['g', 'p', '1'] {
        press(&mut app, ch);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
    for ch in ['g', 'p', '4'] {
        press(&mut app, ch);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 3);
    for ch in ['g', 'g'] {
        press(&mut app, ch);
    }
    for ch in ['g', 'p'] {
        press(&mut app, ch);
    }
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.list.is_none());
    assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
    assert_eq!(fs::read(path).unwrap(), b"%PDF fixture");
}

#[test]
fn pdf_page_picker_rejects_retargeted_pane_and_images() {
    let root = TestRuntimeRoot::new("media-page-picker-stale").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let image = root.join("image.png");
    fs::write(&image, b"PNG fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path).unwrap();
    for ch in ['g', 'p'] {
        press(&mut app, ch);
    }
    app.open_file(image).unwrap();
    let image_buffer = app.active().buffer;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, image_buffer);
    assert!(app.status.contains("PDF page changed"));
    for ch in ['g', 'p'] {
        press(&mut app, ch);
    }
    assert!(app.list.is_none());
    assert!(app.status.contains("requires a PDF"));
}

#[test]
fn pdf_page_navigation_keeps_counted_file_jumps() {
    let root = TestRuntimeRoot::new("media-page-jumps").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].replace_virtual_text("Page 1\nPage 2\nPage 3\nPage 4");
    for (keys, row) in [
        ("3gg", 2),
        ("gg", 0),
        ("ge", 3),
        ("k", 2),
        ("j", 3),
        ("ggG", 3),
    ] {
        for ch in keys.chars() {
            press(&mut app, ch);
        }
        assert_eq!(app.active().cursor(app.active_buffer()).row, row, "{keys}");
    }
}

#[test]
fn paragraph_commands_remain_available_to_configured_keys() {
    let config: Config = serde_yaml::from_str(
        "keys:\n  bind:\n    normal:\n      F7: goto-next-paragraph\n      F8: goto-previous-paragraph\n    select:\n      F7: goto-next-paragraph\n",
    ).unwrap();
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "first\n\nsecond");
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 2);
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
    press(&mut app, 'v');
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 2);
    assert!(!app.active().selection.primary().is_empty());
}
