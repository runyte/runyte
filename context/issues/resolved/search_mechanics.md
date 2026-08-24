---
title: "Search is three inconsistent behaviours sharing one pattern"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 022fbaa
---

## Resolution

Fixed by 022fbaa "Rework search into one consistent mechanism", on top of
9242105 "Replace the generated key table with a curated help card", which the
help changes below build on.

The inconsistency was structural, not cosmetic. `find_search` in `src/app.rs`
scanned line by line with `str::match_indices`, so `/` and `?` were literal and
single-match. `filter_selections`, `select_pattern_in_selections`, and
`select_all_matches` each independently called `Regex::new(&self.search_pattern)`
on the same stored string. One pattern therefore meant two different things
depending on which key read it, and `/foo(` followed by `Alt-k` reported
"invalid regular expression" about text the reporter had typed as plain text.
`matches_in_text` was literal too, which is why the workspace search had no
regular-expression form.

`SearchQuery` replaces the old `search_pattern: String` / `search_forward: bool`
pair and carries the pattern, a `SearchMode`, and the region. `SearchMode::compile`
is the single place a pattern becomes a `Regex`: the two literal flavours run the
pattern through `regex::escape` first, which is what lets someone search for
`foo(` or `a.b` without knowing a regular-expression engine is involved. One
`buffer_matches` free function replaced four ad-hoc match loops; it runs over
`buffer.to_string()` rather than per line so a regular expression can span lines,
and walks a single byte-to-char cursor across the ordered non-overlapping matches
instead of re-counting each prefix.

Matches come back as forward `Range`s — anchor on the first character, head on
the last. The first implementation built them reversed, to put the caret at the
start of each match as the report asked; the reporter tried it and found it
impractical, so the caret sits at the end, where an append or a motion continues
from. This matches what the Vim path already did.

Scoping keys off `operative_span(…) >= 2`. A bare caret is a one-character range
in this grammar, so two characters is what distinguishes "I selected something"
from "my cursor is somewhere". The spans are stored as real selection `Range`s in
`SearchRegion` and mapped through transactions inside `map_transaction_views`,
beside pane selections, for the same reason selections are mapped: they describe
text, not positions, and `n` would otherwise wrap over the wrong span after an
edit. A region belonging to another buffer is ignored rather than honoured.

Three deviations from what the report asked for:

- The report put `Space s` and `Space /` on the workspace searches. During
  planning the reporter redirected those into a `Space s` namespace instead,
  since `s` and `Space s` meaning different things would confuse; the workspace
  searches are `Space s w s`, `Space s w S`, and `Space s w /`, with `Space /`
  kept as the short alias it already was.
- Keeping and removing selections now open their own `keep (regex):` and
  `remove (regex):` prompts, on `Space s k` and `Space s r`. The report did not
  mention them, but once `s` accepts literals the stored pattern is no longer a
  regular expression, so borrowing it was no longer possible. Prompting is what
  Helix does anyway.
- `select-regex`, `split-selection-on-newline`, and `select-all-matches` were
  removed as commands rather than rebound. `/` over a selection is exactly what
  `select-regex` did, `*` absorbed `select-all-matches`, and splitting became
  `Space s e` / `Space s b`, which leave a bare cursor at each line's edge rather
  than a range per line. `search-forward` and `search-backward` had to survive for
  the Vim grammar with no keymap binding, which the inventory audit in `app.rs`
  refuses to classify, hence `GRAMMAR_ONLY_EDITOR_COMMANDS` and
  `CommandExposure::GrammarOnly`.

The Vim grammar is deliberately untouched. `/ ? n N * #` keep directional
single-match semantics and `find_search` keeps its own literal line scan;
sharing `buffer_matches` would have dragged one grammar's meaning into the other.

A later commit finished the move into the namespace. `022fbaa` had kept `Alt-k`
and `Alt-j` as short aliases; every selection command that had an Alt shortcut
is now reached only through `Space s`, and `Space s j` became `Space s r` so the
key matches the word. Removing the Alt pair also ended a real collision:
`src/key_hints.rs` uses `Alt-j` and `Alt-k` to scroll the key-hint popup, so the
popup and the keymap had been competing for them, and which one won depended on
whether the popup happened to be open. `s`, `S`, and `/` remain the only short
spellings.

Tests, all runnable with `cargo test`:

- `src/app.rs`: `search_flavours_fold_case_and_take_literals_literally`,
  `every_match_is_selected_with_the_caret_on_its_last_character`,
  `a_selection_scopes_the_search_and_confines_cycling_to_it`,
  `a_bare_caret_does_not_scope_a_search`,
  `successive_searches_narrow_into_the_previous_matches`,
  `star_selects_every_occurrence_of_the_word_under_the_caret`,
  `filtering_selections_prompts_for_its_own_pattern`,
  `a_failed_search_keeps_the_previous_one_working`,
  `workspace_search_offers_the_same_three_flavours_as_the_buffer`,
  `search_prompt_repeats_and_wraps_unicode_matches`, and the updated
  `command_inventory_classifies_every_command_and_current_binding`.
- `tests/selection.rs`: `splitting_selected_lines_leaves_one_cursor_per_line_edge`,
  `searching_inside_a_selection_selects_every_match_within_it`,
  `matching_filters_keep_or_drop_ranges`,
  `refusing_to_empty_the_selection_is_an_error_not_a_panic`.
- `tests/keymap.rs`: `removed_duplicate_bindings_stay_unbound` covers `?`,
  `Alt-s`, and `Alt-*`.
- `tests/key_hints.rs`:
  `the_search_namespace_and_its_prompts_are_discoverable_on_screen` renders the
  `Space s` popup and each prompt label.

Known limitation: the NORMAL help card is at its ceiling. `tests/key_hints.rs`
requires it to fit an 80×24 terminal, which is exactly sixteen interior lines and
was already full. Fitting search in cost the second overview paragraph, which had
pointed at `Space`, and the `Space w c` row. The card now names the three
flavours and `n`/`N` but not `Space s`, and the scoping rule appears as one
sentence of overview prose rather than being explained. The next addition to that
card has to displace something.

## Report

Search needed to be more consistent. The requested mechanism:

- `s`, `S`, and `/` carry the search actions; `?` is unused.
- `s` searches case-insensitively, `S` case-sensitively, and `/` matches a
  regular expression.
- The prompt reads `search:` for `s` and `search (case-sensitive):` for `S`.
- With no selection, search covers the current buffer. With a selection, it is
  confined to that selection.
- All matches are selected with a multicursor at the start of each match. `n`
  and `N` cycle forward and backward, which collapses the multicursor.
- Cycling with `n`/`N` after a search scoped to a selection must not leave
  that selection.
- `Space /` searches the whole workspace by regular expression, `Space s` by
  string.
- String matching is literal, including special characters, with no wildcards;
  `/` and `Space /` cover anything more expressive.
- The Normal-mode help windows (`:help`, `:?`, `Space ?`) describe the search
  mechanism.
