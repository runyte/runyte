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

#[test]
fn native_startup_restores_first_directory_after_retargeting() {
    let root = TestRuntimeRoot::new("media-launch-directories").unwrap();
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    for name in ["a", "b", "c"] {
        fs::write(first.join(name), "").unwrap();
    }
    let text = root.join("notes.txt");
    fs::write(&text, "notes").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_native_targets(vec![
        LaunchTarget::at(
            &first,
            LaunchPosition {
                line: std::num::NonZeroUsize::new(2).unwrap(),
                column: None,
            },
        ),
        LaunchTarget::new(&second),
        LaunchTarget::new(&text),
    ])
    .unwrap();
    assert_eq!(app.active_buffer().path.as_ref(), Some(&first));
    assert_eq!(app.cursor_position().row, 1);
    assert!(
        app.buffers
            .iter()
            .any(|buffer| buffer.path.as_ref() == Some(&text))
    );
}

#[test]
fn native_pdf_launch_positions_wait_for_metadata_and_apply_once() {
    for (line, pages, expected) in [(2, 4, 1), (20, 4, 3), (2, 1, 0)] {
        let root = TestRuntimeRoot::new("media-launch-pages").unwrap();
        let path = root.join("pages.PDF");
        fs::write(&path, b"%PDF fixture").unwrap();
        let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
        app.native_media = true;
        app.open_native_targets(vec![LaunchTarget::at(
            &path,
            LaunchPosition {
                line: std::num::NonZeroUsize::new(line).unwrap(),
                column: None,
            },
        )])
        .unwrap();
        let id = app.active().buffer;
        assert!(app.launch_positions.contains_key(&id));
        app.update_native_media_pages(&path, 0);
        assert!(app.launch_positions.contains_key(&id));
        app.update_native_media_pages(&path, pages);
        assert_eq!(app.cursor_position().row, expected);
        assert!(!app.launch_positions.contains_key(&id));
        app.active_mut().replace_selection(Selection::point(0));
        app.update_native_media_pages(&path, pages + 1);
        assert_eq!(app.cursor_position().row, 0);
    }
}

#[test]
fn native_background_pdf_launch_position_survives_switches_before_metadata() {
    for reopen in [false, true] {
        let root = TestRuntimeRoot::new("media-background-launch-pages").unwrap();
        let text = root.join("notes.txt");
        let path = root.join("pages.pdf");
        fs::write(&text, "notes").unwrap();
        fs::write(&path, b"%PDF fixture").unwrap();
        let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
        app.native_media = true;
        app.open_native_targets(vec![
            LaunchTarget::new(&text),
            LaunchTarget::at(
                &path,
                LaunchPosition {
                    line: std::num::NonZeroUsize::new(3).unwrap(),
                    column: None,
                },
            ),
        ])
        .unwrap();
        let text_id = app.active().buffer;
        let pdf = app
            .buffers
            .iter()
            .position(|buffer| buffer.media_path.as_ref() == Some(&path))
            .unwrap();
        app.switch_buffer(pdf);
        assert!(app.launch_positions.contains_key(&pdf));
        app.open_file(text.clone()).unwrap();
        app.update_native_media_pages(&path, 4);
        assert_eq!(app.active().buffer, text_id);
        assert_eq!(app.cursor_position().row, 0);
        if reopen {
            app.open_file(path.clone()).unwrap();
        } else {
            app.switch_buffer(pdf);
        }
        assert_eq!(app.cursor_position().row, 2);
        assert!(!app.launch_positions.contains_key(&pdf));
    }
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
    let request = app.media_requests.pop_front().unwrap();
    assert_eq!(request.action, ViewAction::MoveDown);
    assert_eq!(request.page, 1);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
    // At fit size the frontend reports a page motion; zoomed views pan locally.
    app.navigate_native_media(request.pane, &request.path, 1);
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
fn media_q_and_escape_request_the_same_back_action_in_normal_and_select_modes() {
    use crate::media::ViewAction;
    let root = TestRuntimeRoot::new("media-back-alias").unwrap();
    for name in ["pages.pdf", "photo.png"] {
        let path = root.join(name);
        fs::write(&path, b"media fixture").unwrap();
        for select in [false, true] {
            for back in [KeyCode::Char('q'), KeyCode::Escape] {
                let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
                app.native_media = true;
                app.open_file(path.clone()).unwrap();
                if select {
                    press(&mut app, 'v');
                }
                app.media_requests.clear();
                key(&mut app, back, Modifiers::NONE);
                assert_eq!(app.mode, Mode::Normal);
                assert_eq!(app.media_requests.len(), 1);
                let request = app.media_requests.pop_front().unwrap();
                assert_eq!(request.action, ViewAction::Back);
                assert_eq!(request.path, path);
                assert_eq!(request.pane, app.active_pane);
            }
        }
    }
}

#[test]
fn pdf_page_shortcuts_and_counted_vertical_requests_use_media_scope() {
    use crate::media::ViewAction;
    let root = TestRuntimeRoot::new("pdf-navigation-keys").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    app.update_native_media_pages(&path, 4);
    for (next, previous) in [('f', 'b'), ('d', 'u'), ('n', 'p')] {
        key(&mut app, KeyCode::Char(next), Modifiers::CONTROL);
        assert_eq!(app.active().cursor(app.active_buffer()).row, 1);
        key(&mut app, KeyCode::Char(previous), Modifiers::CONTROL);
        assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
    }
    key(&mut app, KeyCode::PageDown, Modifiers::NONE);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 1);
    key(&mut app, KeyCode::PageUp, Modifiers::NONE);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
    for ch in ['3', 'j', 'k'] {
        press(&mut app, ch);
    }
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
    assert_eq!(
        app.media_requests
            .iter()
            .map(|request| request.action)
            .collect::<Vec<_>>(),
        vec![
            ViewAction::MoveDown,
            ViewAction::MoveDown,
            ViewAction::MoveDown,
            ViewAction::MoveUp,
            ViewAction::MoveDown,
            ViewAction::MoveUp
        ]
    );
    app.media_requests.clear();
    press(&mut app, 'g');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(
        app.media_requests.is_empty(),
        "pending keys dismiss before media back"
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(
        app.media_requests.pop_front().unwrap().action,
        ViewAction::Back
    );
    // A terminal attachment to retained media has no native viewport to answer.
    app.native_media = false;
    press(&mut app, 'j');
    assert_eq!(app.active().cursor(app.active_buffer()).row, 1);
    press(&mut app, 'k');
    assert_eq!(app.active().cursor(app.active_buffer()).row, 0);
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
    #[cfg(target_os = "macos")]
    {
        key(&mut app, KeyCode::Char('c'), Modifiers::SUPER);
        assert_eq!(
            app.media_requests.back().unwrap().action,
            ViewAction::CopySelection
        );
    }
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.media_requests.back().unwrap().action, ViewAction::Back);
}

#[test]
fn pdf_back_opens_page_buffer_with_ordinary_motions_and_enter_previews() {
    let root = TestRuntimeRoot::new("media-page-buffer").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].replace_virtual_text("Page 1 of 4\nPage 2 of 4\nPage 3 of 4\nPage 4 of 4");
    assert!(app.active_buffer().display_name().starts_with("[pdf]"));
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(
        app.media_requests.pop_back().unwrap().action,
        crate::media::ViewAction::Back
    );
    // The frontend reports Back only when no native selection remains.
    app.leave_native_media(app.active_pane, &path, 2);
    assert!(app.active().shows_pdf_pages());
    assert!(app.list.is_none());
    assert_eq!(app.key_binding_scope(), BindingScope::PdfPages);
    for ch in ['3', 'g', 'g', 'v', 'k'] {
        press(&mut app, ch);
    }
    assert_eq!(app.mode, Mode::Select);
    assert!(
        app.media_requests.is_empty(),
        "page rows use text selection"
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(
        app.active().shows_pdf_pages(),
        "Select mode cancels before leaving rows"
    );
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, 'G');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.active().shows_pdf_pages());
    assert_eq!(app.active().cursor(app.active_buffer()).row, 3);
    app.leave_native_media(app.active_pane, &path, 4);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.active_buffer().is_directory());
    assert!(!app.closed_buffers.contains(&buffer));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, buffer);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 3);
    assert!(!app.active().shows_pdf_pages());
    assert_eq!(fs::read(path).unwrap(), b"%PDF fixture");
}

#[test]
fn queued_media_back_cannot_leave_another_page_or_document() {
    let root = TestRuntimeRoot::new("media-back-stale").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let image = root.join("image.png");
    fs::write(&image, b"PNG fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    app.leave_native_media(app.active_pane, &path, 2);
    assert!(!app.active().shows_pdf_pages());
    app.open_file(image.clone()).unwrap();
    app.leave_native_media(app.active_pane, &path, 1);
    assert!(!app.active().shows_pdf_pages());
    assert!(app.active_buffer().display_name().starts_with("[image]"));
    app.leave_native_media(app.active_pane, &image, 1);
    assert!(app.active_buffer().is_directory());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.active_buffer().media_path.as_deref(),
        Some(image.as_path())
    );
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
    for (keys, row) in [("3gg", 2), ("gg", 0), ("ge", 3), ("ggG", 3)] {
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

#[test]
fn media_space_e_opens_source_directory_and_selects_the_media_file() {
    let root = TestRuntimeRoot::new("media-source-directory").unwrap();
    let directory = root.join("documents");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("alpha.txt"), "earlier entry").unwrap();
    for name in ["target.pdf", "target.png"] {
        let file = directory.join(name);
        fs::write(&file, b"binary fixture").unwrap();
        let file = file.canonicalize().unwrap();
        let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
        app.native_media = true;
        app.working_directory = root.to_path_buf();
        app.open_file(file.clone()).unwrap();
        let media = app.active().buffer;
        assert!(app.active_buffer().path.is_none());
        assert_eq!(app.active_directory().as_path(), file.parent().unwrap());
        press(&mut app, ' ');
        press(&mut app, 'e');
        assert!(app.active_buffer().is_directory());
        assert_eq!(app.active_buffer().path.as_deref(), file.parent());
        assert_eq!(
            app.selected_directory_entry().unwrap().as_deref(),
            Some(file.as_path())
        );
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        assert_eq!(app.active().buffer, media);
        assert_eq!(app.active_buffer().media_path.as_ref(), Some(&file));
        assert_eq!(app.working_directory, root.to_path_buf());
    }
}

#[test]
fn media_source_directory_depends_on_identity_not_supported_extension() {
    let root = TestRuntimeRoot::new("media-future-directory").unwrap();
    let directory = root.join("documents");
    fs::create_dir(&directory).unwrap();
    let file = directory.join("future-format.custom");
    fs::write(&file, b"future format").unwrap();
    let file = file.canonicalize().unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer] = Buffer::virtual_text("future media", "Page 1");
    app.buffers[buffer].media_path = Some(file.clone());
    app.working_directory = root.to_path_buf();
    press(&mut app, ' ');
    press(&mut app, 'e');
    assert_eq!(app.active_buffer().path.as_deref(), file.parent());
    assert_eq!(
        app.selected_directory_entry().unwrap().as_deref(),
        Some(file.as_path())
    );
}

#[test]
fn retained_page_counts_initialize_reopened_buffers_and_only_change_when_needed() {
    let root = TestRuntimeRoot::new("media-count-reattach").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    app.update_native_media_pages(&path, 6);
    let revision = app.active_buffer().revision();
    app.update_native_media_pages(&path, 6);
    assert_eq!(app.active_buffer().revision(), revision);
    press(&mut app, 'G');
    app.update_native_media_pages(&path, 2);
    assert_eq!(app.active().cursor(app.active_buffer()).row, 1);
    app.execute(parse_colon_command("bc").unwrap()).unwrap();
    app.open_file(path.clone()).unwrap();
    app.update_native_media_pages(&path, 6);
    assert_eq!(app.active_buffer().text().len_lines(), 6);
    app.update_native_media_pages(&path, 0);
    app.update_native_media_pages(&path, 10_001);
    assert_eq!(app.active_buffer().text().len_lines(), 6);
}

#[test]
fn changed_pdf_page_count_preserves_each_panes_page_and_selection() {
    use crate::buffer::Position;
    let root = TestRuntimeRoot::new("media-page-count-width").unwrap();
    let path = root.join("pages.pdf");
    fs::write(&path, b"%PDF fixture").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    app.update_native_media_pages(&path, 9);
    for ch in ['8', 'g', 'g'] {
        press(&mut app, ch);
    }
    let first_pane = app.active_pane;
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, 'v');
    let second_pane = app.active_pane;
    assert_ne!(first_pane, second_pane);
    let anchor = Position { row: 3, col: 2 };
    let head = Position { row: 6, col: 4 };
    let selection = Selection::single(crate::selection::Range::new(
        app.active_buffer().offset_of(anchor),
        app.active_buffer().offset_of(head),
    ));
    app.active_mut().replace_selection(selection);
    for pages in [10, 9] {
        app.update_native_media_pages(&path, pages);
        let buffer = app.active_buffer();
        assert_eq!(app.panes[&first_pane].cursor(buffer).row, 7);
        let selected = app.panes[&second_pane].selection.primary();
        assert_eq!(buffer.position_of(selected.anchor), anchor);
        assert_eq!(buffer.position_of(selected.head), head);
    }
    app.update_native_media_pages(&path, 4);
    assert_eq!(app.panes[&first_pane].cursor(app.active_buffer()).row, 3);
    assert_eq!(app.panes[&second_pane].cursor(app.active_buffer()).row, 3);
}

#[test]
fn native_help_lists_the_window_registry_controls_only_for_window_frontends() {
    let root = TestRuntimeRoot::new("native-window-help").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.open_help();
    assert!(
        !app.active_buffer()
            .text()
            .to_string()
            .contains("Native window")
    );
    app.native_media = true;
    app.open_help();
    let text = app.active_buffer().text().to_string();
    for binding in crate::keymap::native_window::BINDINGS {
        assert!(text.contains(binding.description));
        for key in binding.keys {
            assert!(text.contains(&key.label()));
        }
    }
}
