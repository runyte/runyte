// SPDX-License-Identifier: MPL-2.0

use runyte::{
    config::Config,
    settings::{SettingId, render_settings_page},
};
use unicode_width::UnicodeWidthStr;

#[test]
fn settings_keep_complete_keys_values_and_descriptions_on_single_lines() {
    let config = Config::default();
    let values = SettingId::ALL
        .iter()
        .map(|&setting| (setting, setting.configured_value(&config).to_string()))
        .collect::<Vec<_>>();
    let page = render_settings_page(&values);
    let lines = page.text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), values.len() + 2);
    assert_eq!(page.rows.len(), lines.len());
    let value_column = lines[0].find("Saved value").unwrap();
    let description_column = lines[0].find("Description").unwrap();
    assert!(value_column < description_column);
    for (index, (setting, value)) in values.iter().enumerate() {
        let line = lines[index + 2];
        assert_eq!(page.rows[index + 2], Some(*setting));
        assert_eq!(line[..value_column].trim_end(), setting.descriptor().key);
        assert_eq!(line[value_column..description_column].trim_end(), value);
        assert_eq!(
            &line[description_column..],
            setting.descriptor().description
        );
        assert_eq!(line.trim_end(), line, "no padding after the description");
    }
    assert_eq!(
        value_column,
        values
            .iter()
            .map(|(id, _)| id.descriptor().key.width())
            .max()
            .unwrap()
            + 2
    );
}

#[test]
fn long_unicode_values_align_by_cells_and_colour_by_character_offsets() {
    let values = [
        (
            SettingId::Theme,
            "海辺のテーマ-cafe\u{301}-long-name".to_owned(),
        ),
        (SettingId::EditorTabWidth, "4".to_owned()),
    ];
    let page = render_settings_page(&values);
    let lines = page.text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 4);
    let description_column = lines[0].find("Description").unwrap();
    let value_column = lines[0].find("Saved value").unwrap();
    assert_eq!(description_column - value_column, values[0].1.width() + 2);
    for (index, (id, value)) in values.iter().enumerate() {
        let line = lines[index + 2];
        let description_start = line.find(id.descriptor().description).unwrap();
        assert_eq!(line[..description_start].width(), description_column);
        assert!(line.contains(value));
    }
    let coloured = page
        .spans
        .iter()
        .map(|span| {
            (
                page.text
                    .chars()
                    .skip(span.from)
                    .take(span.to - span.from)
                    .collect::<String>(),
                span.scope.name(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        coloured,
        vec![
            ("theme".to_owned(), "function"),
            (values[0].1.clone(), "constant"),
            ("editor.tab_width".to_owned(), "function"),
            ("4".to_owned(), "constant"),
        ]
    );
}

#[test]
fn empty_values_and_empty_registry_leave_readable_headers_without_empty_spans() {
    let empty = render_settings_page(&[]);
    assert_eq!(empty.text.lines().count(), 2);
    assert_eq!(empty.rows, vec![None, None]);
    assert!(empty.spans.is_empty());

    let page = render_settings_page(&[(SettingId::Theme, String::new())]);
    assert_eq!(page.text.lines().count(), 3);
    assert_eq!(page.spans.len(), 1);
    assert_eq!(page.spans[0].scope.name(), "function");
    assert!(
        page.text
            .lines()
            .last()
            .unwrap()
            .ends_with(SettingId::Theme.descriptor().description)
    );
}
