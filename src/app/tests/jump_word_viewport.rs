// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn goto_word_long_unwrapped_suffix_preserves_scrolled_tab_and_wide_cell_labels() {
    for (prefix, scroll, width, targets) in [
        ("aa bb cc ", 0, 4, vec![0, 3]),
        ("x\t aa bb cc ", 1, 6, vec![3]),
        ("zz 界界 aa bb cc ", 3, 7, vec![6]),
        ("xxaayy bb cc ", 2, 5, vec![]),
        ("aa 界界 bb ", 0, 5, vec![0]),
    ] {
        let mut labels = Vec::new();
        for suffix in [String::new(), "zz ".repeat(5000)] {
            let mut app = App::new(Config::default(), None).unwrap();
            app.config.editor.soft_wrap = false;
            seed(&mut app, &format!("{prefix}{suffix}"));
            app.active_mut().scroll_col = scroll;
            app.active_mut().wrap_width = width;
            app.label_visible_words();
            assert_eq!(app.jump.as_ref().map_or(0, JumpLabels::len), targets.len());
            for offset in &targets {
                assert!(app.jump.as_ref().unwrap().label_at(*offset).is_some());
            }
            labels.push(app.jump);
        }
        assert_eq!(labels[0], labels[1], "{prefix:?}");
    }
}
