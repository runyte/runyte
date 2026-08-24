---
title: "E skips past the current word when the row is followed by an empty line"
status: resolved
reported: 2026-07-31
resolved: 2026-07-31
legacy_commit: b549568
---

## Resolution

Fixed in commit `b549568`, "Treat a line break as a word boundary".

`class_at` in `src/app.rs` mapped a line terminator to `None`, and the
`offsets_after`/`offsets_before` walk it was paired with filtered terminators
out of the offset stream entirely. Both are correct for the Normal-mode
cursor, which cannot rest on a newline, and `word_end` inherited them. The
result was that word motion never saw a line break at all: the last word of a
row and the first word of the next non-empty row scanned as one continuous run
of word characters, so `E` on `alpha` in `alpha\n\nbravo` walked straight
through the blank row and stopped inside `bravo`.

The empty line in the report is not what triggers it — a single newline does
the same thing — but it is what makes the jump visible, because the cursor
lands two rows down instead of one.

`w`/`W` had the same cause and a worse symptom: from a word at the end of a
row, `word_forward_kind` also failed to see a class change and ran to the end
of the document, skipping the next row's first word. The same change fixes it.

Word motion now scans through its own `word_class_at`, `word_scan_next`, and
`word_scan_previous`, which include line terminators and class them as
whitespace, so a line break ends a word the way a space does. `is_word_start`,
`word_forward_kind`, `word_back_kind`, and `word_end` use them. Each of those
functions only ever returns an offset whose class is non-zero, so the cursor
still cannot land on a terminator and the original reason for hiding newlines
is preserved. `find_character` (`f`/`t`) and the row-confined `word_bounds`
keep the old newline-hiding helpers, since their behavior depends on them.

Deviation from Helix: Helix stops `w` on an empty row, treating the row as a
word of its own. Runyte does not, because no motion here puts the cursor on a
position that holds no character. `w` from `alpha` in `alpha\n\nbravo` goes to
`bravo`, not to the blank row.

Covered by `word_end_stops_at_the_end_of_the_row_it_started_on`,
`word_end_from_a_word_end_crosses_blank_rows_to_the_next_word_end`, and
`word_forward_from_the_last_word_of_a_row_lands_on_the_next_rows_word` in
`src/app.rs`, alongside the existing
`word_and_character_motions_handle_unicode_and_lines` and
`word_back_from_an_empty_final_row_stops_on_the_previous_line`.

## Report

`E` should move to the last character of the current word. It mostly did, but
when the word was followed by an empty line the cursor moved instead to the
beginning of the next word, two lines below.
