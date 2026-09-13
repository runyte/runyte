// SPDX-License-Identifier: MPL-2.0

use super::*;

fn offset(text: &str, needle: &str) -> usize {
    text[..text.find(needle).unwrap()].chars().count()
}

#[test]
fn goto_file_follows_markdown_image_labels_in_source_and_rendered_pages() {
    let root = temporary("markdown-image-navigation");
    let directory = root.join("notes");
    fs::create_dir_all(&directory).unwrap();
    let image = directory.join("hé界llo (1).png");
    fs::write(&image, b"\x89PNG\r\n\x1a\n\0").unwrap();
    let source = directory.join("note.md");
    for (markdown, label) in [
        ("[Image 1](<hé界llo (1).png>)", "Image 1"),
        ("![**Image 1**](<hé界llo (1).png>)", "Image 1"),
        ("![](<hé界llo (1).png>)", "hé界llo (1).png"),
        (
            "| Picture |\n| --- |\n| [Image 1](<hé界llo (1).png>) |",
            "Image 1",
        ),
    ] {
        fs::write(&source, markdown).unwrap();
        for rendered in [false, true] {
            let mut app = App::new(Config::default(), Some(source.clone())).unwrap();
            app.project_root = root.clone();
            app.programs = external_open::ProgramCache::load(None);
            if rendered {
                press(&mut app, '?');
            }
            let page = app.active().buffer;
            let start = offset(&text(&app), label);
            for column in 0..label.chars().count() {
                app.active_mut()
                    .replace_selection(Selection::point(start + column));
                press(&mut app, 'g');
                press(&mut app, 'f');
                assert_eq!(
                    app.external_target.as_ref(),
                    Some(&image),
                    "{markdown}, rendered={rendered}, column={column}"
                );
                assert_eq!(app.active().buffer, page);
                key(&mut app, KeyCode::Escape, Modifiers::NONE);
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn goto_file_follows_markdown_web_labels_and_keeps_explicit_selections_exact() {
    let root = temporary("markdown-web-navigation");
    fs::create_dir_all(&root).unwrap();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let mut app = App::new(Config::default(), None).unwrap();
    // Keep selected "Docs" away from the checkout's "docs" on case-insensitive filesystems.
    app.project_root = root.clone();
    let recorded = Arc::clone(&opened);
    app.ports.browser = Box::new(move |url| {
        recorded.lock().unwrap().push(url.to_owned());
        Ok(())
    });
    seed(
        &mut app,
        "界 [**Docs**](https://example.com/a_(b)?q=1#part) and [Docs](www.example.org)\n",
    );
    let source = app.active().buffer;
    for rendered in [false, true] {
        if rendered {
            press(&mut app, '?');
        }
        let current = text(&app);
        for start in [
            offset(&current, "Docs"),
            current[..current.rfind("Docs").unwrap()].chars().count(),
        ] {
            app.active_mut()
                .replace_selection(Selection::point(start + 1));
            press(&mut app, 'g');
            press(&mut app, 'f');
        }
        app.active_mut()
            .replace_selection(Selection::single(Range::new(
                offset(&current, "Docs"),
                offset(&current, "Docs") + 3,
            )));
        press(&mut app, 'g');
        press(&mut app, 'f');
        assert!(
            app.displayed_status_message()
                .contains("path not found: Docs")
        );
    }
    assert_eq!(
        *opened.lock().unwrap(),
        [
            "https://example.com/a_(b)?q=1#part",
            "https://www.example.org",
            "https://example.com/a_(b)?q=1#part",
            "https://www.example.org"
        ]
    );
    assert_ne!(app.active().buffer, source);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn markdown_link_inference_respects_inline_boundaries() {
    for literal in [
        "`[label](file.txt)`",
        "\\[label](file.txt)",
        "[label](<bad<path>)",
        "[label](unfinished",
        "[label](file.txt) after",
    ] {
        let caret = if literal.ends_with("after") {
            literal.chars().count() - 1
        } else {
            offset(literal, "label")
        };
        assert_eq!(
            crate::markdown::link_under_cursor(literal, caret),
            None,
            "{literal}"
        );
    }
    assert_eq!(
        crate::markdown::link_under_cursor("[label](file.txt \"title\")", 3).as_deref(),
        Some("file.txt")
    );
    assert_eq!(
        crate::markdown::link_under_cursor("[label](a_(b).txt)", 3).as_deref(),
        Some("a_(b).txt")
    );
}

#[test]
fn goto_file_from_rendered_markdown_uses_the_source_directory_after_edits() {
    let root = temporary("markdown-file-navigation");
    let directory = root.join("notes");
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("target.txt");
    fs::write(&target, "destination\n").unwrap();
    let source_path = directory.join("note.md");
    fs::write(&source_path, "[the file](target.txt)\n\ntarget.txt\n").unwrap();
    let mut app = App::new(Config::default(), Some(source_path)).unwrap();
    app.project_root = root.clone();
    let source = app.active().buffer;
    for rendered in [false, true] {
        app.switch_buffer(source);
        if rendered {
            press(&mut app, '?');
            app.apply_to_buffer(source, &Transaction::insert(0, "A new paragraph\n\n"));
        }
        let page = app.active().buffer;
        let current = text(&app);
        let label = offset(&current, "the file") + 2;
        let bare_path = current[..current.rfind("target.txt").unwrap()]
            .chars()
            .count();
        for position in [label, bare_path] {
            app.switch_buffer(page);
            app.active_mut()
                .replace_selection(Selection::point(position));
            press(&mut app, 'g');
            press(&mut app, 'f');
            assert_eq!(app.active_buffer().path.as_ref(), Some(&target));
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn markdown_toggle_tracks_characters_through_block_and_inline_formatting() {
    let examples = [
        ("# A **hé界llo** heading\n", "hé界llo"),
        ("A **hé界llo** heading\n===\n", "hé界llo"),
        ("- [x] **hé界llo** item\n", "hé界llo"),
        ("> > A *hé界llo* quote\n", "hé界llo"),
        ("```rust\nlet hé界llo = 1;\n```\n", "hé界llo"),
        ("    let hé界llo = 1;\n", "hé界llo"),
        ("A **nested *hé界llo* phrase**\n", "hé界llo"),
        ("A ~~hé界llo~~ phrase\n", "hé界llo"),
        ("A ` hé界llo ` span\n", "hé界llo"),
        ("A [hé界llo](target.txt) link\n", "hé界llo"),
        ("A [label](<hé界llo.txt>) link\n", "hé界llo"),
        ("A ![](folder/hé界llo.png) image\n", "hé界llo"),
        ("A <https://hé界llo.test> link\n", "hé界llo"),
        ("A \\*hé界llo\\* phrase\n", "hé界llo"),
        ("First line **with\n  hé界llo** here\n", "hé界llo"),
        (
            "| Name | Value |\n| --- | --- |\n| x | **hé界llo** |\n",
            "hé界llo",
        ),
        (
            "| Name | Value |\n| --- | --- |\n| x | a\\|hé界llo |\n",
            "hé界llo",
        ),
        ("---\nname: hé界llo\n---\nBody\n", "hé界llo"),
        ("<!-- hé界llo -->\n", "hé界llo"),
        ("# Earlier\r\n\r\n## hé界llo\r\n", "hé界llo"),
    ];
    for (source, needle) in examples {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, source);
        let document = app.active().buffer;
        for column in 0..needle.chars().count() {
            let source_offset = offset(source, needle) + column;
            app.active_mut()
                .replace_selection(Selection::point(source_offset));
            press(&mut app, '?');
            assert_eq!(
                app.active().head(),
                offset(&text(&app), needle) + column,
                "{source:?}"
            );
            // Also exercise the page's map without the exact-origin shortcut.
            app.active_mut().markdown_origin = None;
            press(&mut app, '?');
            assert_eq!(app.active().buffer, document);
            assert_eq!(app.active().head(), source_offset, "{source:?}");
        }
    }
}

#[test]
fn moving_on_a_rendered_page_returns_to_that_occurrence_in_source() {
    let source = "# repeat\n\nrepeat **repeat** repeat\n\n## repeat\n\nfinal **destination**\n";
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, source);
    let source_offset = offset(source, "repeat**") + 2;
    app.active_mut()
        .replace_selection(Selection::point(source_offset));
    press(&mut app, '?');
    assert_eq!(
        app.active().head(),
        offset(&text(&app), "repeat repeat repeat") + 9
    );
    let destination = offset(&text(&app), "destination") + 3;
    app.active_mut()
        .replace_selection(Selection::point(destination));
    press(&mut app, 'l');
    press(&mut app, '?');
    assert_eq!(app.active().head(), offset(source, "destination") + 4);
}

#[test]
fn markdown_mapping_follows_source_edits_and_is_replaced_on_rerender() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "# Title\n\n**destination**\n");
    let source = app.active().buffer;
    press(&mut app, '?');
    let page = app.active().buffer;
    let destination = offset(&text(&app), "destination") + 3;
    app.active_mut()
        .replace_selection(Selection::point(destination));
    app.apply_to_buffer(source, &Transaction::insert(0, "new paragraph\n\n"));
    press(&mut app, '?');
    assert_eq!(app.active().head(), offset(&text(&app), "destination") + 3);
    press(&mut app, '?');
    assert_eq!(app.active().buffer, page);
    assert_eq!(app.active().head(), offset(&text(&app), "destination") + 3);
    app.close_buffer(page);
    assert!(!app.markdown_positions.contains_key(&page));
}

#[test]
fn markdown_positions_handle_empty_pages_and_removed_or_generated_characters() {
    for source in [
        "",
        "\n\n",
        "# Heading\n\n---\n",
        "```\n```\n",
        "| A | B |\n| --- | --- |\n",
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, source);
        for source_offset in 0..=source.chars().count() {
            app.active_mut()
                .replace_selection(Selection::point(source_offset));
            press(&mut app, '?');
            assert!(app.active().head() <= app.active_buffer().len_chars());
            let page_len = app.active_buffer().len_chars();
            app.active_mut()
                .replace_selection(Selection::point(page_len / 2));
            press(&mut app, '?');
            assert!(app.active().head() <= source.chars().count());
        }
    }
}

#[test]
fn unchanged_markdown_toggle_restores_even_removed_source_markup() {
    let source = "# Heading\n\nSome **bold** text\n\n```rust\ncode\n```\n";
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, source);
    for position in 0..source.chars().count() {
        app.active_mut()
            .replace_selection(Selection::point(position));
        press(&mut app, '?');
        press(&mut app, '?');
        assert_eq!(app.active().head(), position);
    }
}

#[test]
fn markdown_mapping_handles_replacement_deletion_and_undo_in_source() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "# Heading\n\n**abcdef**\n");
    let source = app.active().buffer;
    press(&mut app, '?');
    let page = app.active().buffer;
    let page_start = offset(&text(&app), "abcdef");
    let source_start = offset(&app.buffers[source].to_string(), "abcdef");
    app.apply_to_buffer(
        source,
        &Transaction::change(source_start + 1, source_start + 4, "XYZW"),
    );
    for (column, expected) in [(0, 0), (1, 1), (2, 5), (3, 5), (4, 5), (5, 6)] {
        assert_eq!(
            app.markdown_positions[&page].to_source(page_start + column),
            source_start + expected
        );
    }
    app.apply_to_buffer(
        source,
        &Transaction::delete(source_start + 1, source_start + 5),
    );
    assert_eq!(
        app.markdown_positions[&page].to_source(page_start + 5),
        source_start + 2
    );
    app.switch_buffer(source);
    app.undo();
    assert_eq!(
        app.markdown_positions[&page].to_source(page_start + 5),
        source_start + 6
    );
    app.undo();
    assert_eq!(
        app.markdown_positions[&page].to_source(page_start + 5),
        source_start + 5
    );
}

#[test]
fn markdown_toggle_keeps_the_destination_visible_after_wrapping_and_resize() {
    let source = format!(
        "{}\n# Destination\n\nA **long phrase with several words** at the end.\n",
        "Earlier paragraph.\n\n".repeat(50)
    );
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    config.editor.scroll_offset = 0;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, &source);
    let position = offset(&source, "several");
    app.active_mut()
        .replace_selection(Selection::point(position));
    for width in [24, 60] {
        for _ in 0..2 {
            press(&mut app, '?');
            let area = Rect {
                x: 0,
                y: 0,
                width,
                height: 12,
            };
            let view = app.prepare_view(FrameGeometry {
                screen: area,
                editor: area,
                ..FrameGeometry::default()
            });
            let snapshot = app.snapshot(&view);
            assert!(snapshot.pane(0).unwrap().cursor_screen_row.is_some());
            assert_eq!(app.active().head(), offset(&text(&app), "several"));
            assert!(app.active().scroll_row > 0);
        }
    }
}

#[test]
fn markdown_maps_crlf_paragraph_breaks_to_the_source_newline() {
    let mut app = App::new(Config::default(), None).unwrap();
    let source = "**first\r\nsecond**\r\n";
    seed(&mut app, source);
    press(&mut app, '?');
    let page = app.active().buffer;
    assert_eq!(
        app.markdown_positions[&page].to_source(offset(&text(&app), "\n")),
        offset(source, "\n")
    );
}
