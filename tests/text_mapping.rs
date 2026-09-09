// SPDX-License-Identifier: MPL-2.0

use runyte::text::{Assoc, Change, Text, Transaction};

#[test]
fn unicode_mapping_and_inverse_keep_normalized_character_coordinates() {
    let mut text = Text::from_str("ab猫cd");
    let transaction = Transaction::new(vec![Change::new(4, 5, "😀界"), Change::new(1, 3, "é")]);
    assert_eq!(transaction.footprint(), 3);
    assert_eq!(transaction.map_offset(2, Assoc::Before), 1);
    assert_eq!(transaction.map_offset(2, Assoc::After), 2);
    assert_eq!(transaction.map_offset(4, Assoc::After), 3);
    assert_eq!(transaction.map_offset(5, Assoc::After), 5);
    let inverse = text.apply(&transaction.clone()).into_transaction();
    assert_eq!(text.to_string(), "aéc😀界");
    assert_eq!(inverse.footprint(), 3);
    assert_eq!(inverse.map_offset(2, Assoc::After), 3);
    assert_eq!(inverse.map_offset(4, Assoc::Before), 4);
    assert_eq!(inverse.map_offset(4, Assoc::After), 5);
    text.apply(&inverse);
    assert_eq!(text.to_string(), "ab猫cd");
}

#[test]
fn authored_change_mutation_and_overlap_rejection_precede_mapping_preparation() {
    let mut change = Change::new(1, 3, "x");
    change.from = 0;
    change.to = 2;
    change.text = "🦀é".into();
    let transaction = Transaction::new(vec![Change::new(1, 4, "ignored"), change]);
    assert_eq!(transaction.changes().len(), 1);
    assert_eq!(transaction.footprint(), 2);
    assert_eq!(transaction.map_offset(1, Assoc::Before), 0);
    assert_eq!(transaction.map_offset(1, Assoc::After), 2);
    let mut text = Text::from_str("abcd");
    let inverse = text.apply(&transaction).into_transaction();
    assert_eq!(text.to_string(), "🦀écd");
    text.apply(&inverse);
    assert_eq!(text.to_string(), "abcd");
}

#[test]
fn coincident_unicode_insertions_keep_association_and_inverse_deletions() {
    let transaction = Transaction::new(vec![Change::new(1, 1, "é"), Change::new(1, 1, "猫")]);
    assert_eq!(transaction.footprint(), 2);
    assert_eq!(transaction.map_offset(1, Assoc::Before), 1);
    assert_eq!(transaction.map_offset(1, Assoc::After), 3);
    let mut text = Text::from_str("ab");
    let inverse = text.apply(&transaction).into_transaction();
    assert_eq!(text.to_string(), "aé猫b");
    assert_eq!(inverse.footprint(), 0);
    assert_eq!(inverse.map_offset(2, Assoc::After), 1);
    text.apply(&inverse);
    assert_eq!(text.to_string(), "ab");
}
