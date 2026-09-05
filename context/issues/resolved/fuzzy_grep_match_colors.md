---
title: "Fuzzy grep did not distinguish direct substrings from fuzzy subsequences"
status: resolved
reported: 2026-08-16
resolved: 2026-08-16
legacy_commit: c5c923e
---

## Resolution

Commit `c5c923e` (`Distinguish direct fuzzy grep matches`) resolves the issue.
`ui::draw_picker` previously rendered every emphasized preview character as
bold accent-coloured text, so the fuzzy scorer's contiguous and gapped
alignments looked identical and did not share the visual grammar of buffer
search selections. The workspace-frame snapshot also flattened content
snippets into plain text, which discarded their emphasis positions before an
attached client could render them.

`file_picker::is_direct_match` now classifies the scorer's ordered character
positions: a non-empty run of consecutive positions is a direct substring,
while any gap is a fuzzy subsequence. The shared `ui::fuzzy_preview_lines`
renderer fills direct substrings with `fuzzy_match_primary` and the individual
characters of fuzzy subsequences with `fuzzy_match_secondary`. Both standalone
and attached rendering use that function. Semantic snippet previews now cross
the private workspace protocol with their source rows, focused row, and match
positions intact; the protocol moved to version 10 because older peers cannot
render that frame shape.

The theme schema gained the optional `fuzzy_match_primary` and
`fuzzy_match_secondary` colours. They fall back to `selection_primary` and
`selection`, respectively, so bundled and existing custom themes immediately
use the same warm primary and cool secondary colours as `Space s`, while a
custom theme may override either fuzzy-grep role independently.

Coverage is provided by
`file_picker::tests::matching_is_smart_case_and_reports_character_positions`
in `src/file_picker.rs` for direct and fuzzy classification;
`config::tests::custom_theme_cursor_colors_fall_back_to_its_accent` in
`src/config.rs` for theme inheritance and overrides;
`ui::tests::fuzzy_grep_picker_keeps_paths_in_the_list_and_content_in_the_preview`
in `src/ui.rs` for the two terminal backgrounds;
`app::tests::fuzzy_grep_searches_contents_at_both_roots_and_enter_jumps_to_the_match`
in `src/app.rs` for semantic snippet snapshots; and
`protocol::tests::fuzzy_grep_snippet_round_trips_match_positions_over_the_wire`
in `src/protocol/mod.rs` for transport preservation.

Known limitation: fuzzy grep still matches within one line at a time and uses
the scorer's existing smart-case comparison; it does not form a match across a
line boundary.

## Report

Fuzzy grep showed matching substrings as coloured text. A screenshot showed a
query for `example` whose matching author text used the accent foreground.

Direct, one-to-one substring matches should use the primary-selection colour,
and non-contiguous fuzzy matches should use the secondary-selection colour,
matching the colours used by `Space s`. A second screenshot showed those
primary and secondary search-selection backgrounds in a file buffer.

The two fuzzy-grep match colours should be named theme colours so custom themes
can configure them.
