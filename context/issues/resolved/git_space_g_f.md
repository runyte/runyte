---
title: "Space g f did not fuzzy-search Git commit messages"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: e90f9d3
---

## Resolution

Commit e90f9d3 (`Add fuzzy commit message search`) added the command because the
Git namespace had no commit-search entry point: `open_git_log` could only open
bounded history pages, and the shared `ListPicker` could only substring-filter
its rows. `GitProvider::search_commits` now returns a bounded typed result made
from machine-delimited Git output, retaining each full commit message as the
search haystack and its full object ID as the selection identity. The operation
runs through the existing asynchronous Git service, so history discovery does
not block editor input.

`ListPicker` gained an opt-in fuzzy mode that uses the native file-picker scorer
while leaving existing symbol, buffer, setting, and result pickers on their
current substring behavior. `Space g f` and `:git-search-commits` open this mode
over the newest 5,000 commits reachable from `HEAD`; Enter opens the selected
object through the same bounded commit-detail path used by Git log and blame.
This is a deliberate Runyte addition: Helix has no `Space g` namespace.

A later presentation fix gave that picker the same two-column shape as the
file and content pickers. Its left column now keeps the abbreviated object ID
beside the subject and trims the row to the available width. Its right column
shows the author, author date, and full retained message. Matches in that
preview reuse fuzzy grep's selection roles: a contiguous substring has the
primary match background, while a non-contiguous subsequence has the secondary
background. The owned overlay snapshot carries the same semantic match spans,
so an attached TUI does not redraw the picker differently from a standalone
one.

Coverage:

- `picker::tests::fuzzy_filter_matches_ordered_non_contiguous_characters_and_ranks_them`
  in `src/picker.rs`
- `picker::tests::preview_matches_distinguish_direct_text_from_fuzzy_subsequences`
  in `src/picker.rs`
- `git::history::tests::commit_search_keeps_the_full_message_beside_typed_summary_fields`
  in `src/git/history.rs`
- `app::tests::commit_message_picker_fuzzy_matches_bodies_and_keeps_object_identity`
  in `src/app.rs`
- `ui::tests::commit_picker_renders_list_and_message_with_exact_and_fuzzy_selections`
  in `src/ui.rs`
- `protocol::tests::matched_text_preview_round_trips_match_positions_over_the_wire`
  in `src/protocol/mod.rs`
- `commit_search_returns_full_messages_in_newest_first_order` in
  `tests/git_provider.rs`
- `nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths` in
  `tests/keymap.rs`

Known limitation: search is intentionally limited to the newest 5,000 commits
reachable from `HEAD`, and the picker labels when that bound is reached.

## Report

`Space g f` should fuzzy-search Git commit messages. It should work similarly
to the fuzzy file picker and fuzzy grep.
