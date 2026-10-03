// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn capped_index_keeps_frequent_words_after_the_distinct_word_limit() {
    let mut source = String::new();
    for index in 0..MAX_WORDS_PER_BUFFER + 100 {
        source.push_str(&format!("word{index:05} "));
    }
    source.push_str("zebra zebra zebra café café");
    let words = BufferWords::from_text(&Text::from_str(&source));
    assert_eq!(words.entries().len(), MAX_WORDS_PER_BUFFER);
    assert_eq!(words.entries.capacity(), MAX_WORDS_PER_BUFFER);
    assert_eq!(words.entries()[0], ("zebra".to_owned(), 3));
    assert_eq!(words.entries()[1], ("café".to_owned(), 2));
    assert_eq!(words.entries()[2], ("word00000".to_owned(), 1));
    assert_eq!(
        words.entries().last(),
        Some(&(format!("word{:05}", MAX_WORDS_PER_BUFFER - 3), 1))
    );
}

#[test]
fn snapshots_share_unchanged_word_lists_across_updates_and_removals() {
    let handle = spawn();
    handle.notify_update(1, Text::from_str("retained retained"));
    handle.notify_update(2, Text::from_str("original"));
    handle.flush();
    let initial = handle.current();

    handle.notify_update(2, Text::from_str("replacement"));
    handle.flush();
    let updated = handle.current();
    assert!(Arc::ptr_eq(&initial.buffers[&1], &updated.buffers[&1]));
    assert!(!Arc::ptr_eq(&initial.buffers[&2], &updated.buffers[&2]));
    assert_eq!(
        initial.buffer_words(2).unwrap().entries(),
        &[("original".to_owned(), 1)]
    );
    assert_eq!(
        updated.buffer_words(2).unwrap().entries(),
        &[("replacement".to_owned(), 1)]
    );

    handle.notify_remove(2);
    handle.flush();
    let removed = handle.current();
    assert!(Arc::ptr_eq(&initial.buffers[&1], &removed.buffers[&1]));
    assert!(removed.buffer_words(2).is_none());
}

#[test]
fn streaming_word_counts_preserve_unicode_hyphens_and_line_boundaries() {
    let prefix = " ".repeat(1023);
    let text = Text::from_str(&format!(
        "{prefix}café-au-lait café-au-lait\r\n中文-数字 中文-\n数字 foo--bar"
    ));
    let words = BufferWords::from_text(&text);
    assert_eq!(
        words.entries(),
        &[
            ("café-au-lait".to_owned(), 2),
            ("bar".to_owned(), 1),
            ("foo".to_owned(), 1),
            ("中文".to_owned(), 1),
            ("中文-数字".to_owned(), 1),
            ("数字".to_owned(), 1),
        ]
    );
}
