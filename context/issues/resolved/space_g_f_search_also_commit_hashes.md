---
title: "Space g f matched only commit messages, not hashes, authors, or dates"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 5756390
---

## Resolution

Fixed in `5756390` ("Search commits by object ID, author, and date").

`App::open_git_commit_search_result` built each picker row with
`PickerItem::searchable`, whose third argument replaces the default
label-plus-detail haystack outright. It passed `CommitSearchEntry::message`,
so the fuzzy filter saw the commit message and nothing else. The row itself
already displayed the author date, the author, and the abbreviated object ID,
which meant none of the three could be typed to find the commit they belonged
to.

`CommitSearchEntry::haystack` in `src/git/history.rs` now assembles that text
instead: the trimmed message, then the full object ID, the author, and the
author date. It lives beside the parsing of those fields rather than in
`app.rs` so that what the picker matches stays a property of the commit
record. Two details are deliberate. The fields are joined with spaces because
`file_picker::fuzzy_match` awards a word-start bonus to a character preceded
by `/`, `_`, `-`, `.`, or a space, so each field's first character scores as
the start of a token. Only the full object ID appears, not the abbreviated
one as well: the abbreviation is a prefix of the full ID, so typing the twelve
characters a row shows already matches, and repeating them would only add hex
characters for unrelated queries to match against.

Matching stays fuzzy rather than switching to a prefix or exact test for
ID-shaped queries. A wrong-but-lower-scoring commit can still appear below the
right one when a query is a scattered subsequence of another commit's text,
which was already true of message matching and keeps one filter rule for the
whole picker.

The command description, `README.md`, and
`context/reference/helix-keymap-v1.md` were updated to say what the picker now
searches.

Tests:

- `the_search_haystack_carries_identity_beside_the_message` in
  `src/git/history.rs` pins the assembled haystack for a parsed commit.
- `commit_picker_also_matches_object_ids_authors_and_dates` in `src/app.rs`
  filters a two-commit picker by abbreviated ID, full ID, author, and author
  date, and asserts each query leaves exactly the one row that field belongs
  to.
- `commit_message_picker_fuzzy_matches_bodies_and_keeps_object_identity` in
  `src/app.rs` continues to cover matching against a message body.

Known limitation: ref decorations — branch and tag names — are still not part
of the haystack, and the picker searches only the newest 5,000 commits
reachable from `HEAD`, so an object ID outside that window is not found.

## Report

`Space g f` searched only commit messages. It should search commit hashes,
authors, and dates as well.
