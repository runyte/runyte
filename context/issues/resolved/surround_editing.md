---
title: "No commands add, replace, or delete the pair around a selection"
status: resolved
reported: 2026-10-07
resolved: 2026-10-07
commit: fc4e3b9
---

## Resolution

fc4e3b9 (`Add m s, m r and m d surround editing`) adds three commands,
`surround-add`, `surround-replace`, and `surround-delete`, bound to `m s`,
`m r`, and `m d` in Normal and Select modes. Runyte already had the `m i`/`m a`
pair objects but nothing that edited a pair in place.

The pairs are operands rather than further keys of the sequence, as `r` takes
its replacement. Explicit bindings per pair, the shape `m i (` uses, were not
viable for `m r`: eight source pairs times seven replacements would have
needed fifty-six commands. The input grammar, which could only collect one
character operand, now collects a second for commands whose
`takes_second_character` is true. `RunyteGrammar` holds the first operand in
`first_operand` and reports `GrammarNotice::AwaitingSecondCharacter`, which
the status line shows as `replace ( with …`; Escape between the operands
abandons both. `CommandExecutionContext` carries the second operand, and
`validate_editor_execution` rejects an invocation that has one too few or too
many. The test-only `VimGrammar` follows the same rule through
`VimAwaiting::NamespaceSecondCharacter`.

`src/app/surround.rs` performs the edits. `m s` wraps each operative span, so
a bare caret wraps the character under it, and selects the result with its
delimiters in Normal mode. Either bracket of `()`, `[]`, `{}`, or `<>` names
that pair; any other character is used on both sides. Overlapping operative
spans, which inclusive selections can produce from touching ranges, are
wrapped together so the computed selection matches the inserted text.

A follow-up review corrected primary-selection mapping in `surround_add`.
Combining operative spans had retained the original primary index, which
could identify a later, unrelated result, and kept the first range's direction
even when a later range was primary. The merge now maps the primary index to
its combined span and preserves that range's direction.

`m r` and `m d` look the pair up from the character under each caret rather
than from the whole selection. `enclosing_pair` deliberately skips a pair
equal to the requested range so that repeating `m a` grows outward; querying
the selection after `m a (` would therefore have edited the next pair out.
The per-range lookup that `select_delimiter` performed is now
`structural_selection::enclosing_delimiter`, shared by both, so selection and
surround editing cannot disagree about which pair encloses a caret: the syntax
tree answers first, a balanced text scan otherwise. The pair's delimiters are
the first and last characters of the around span, which both resolutions
guarantee. Every caret must find a pair before anything changes; carets
inside the same pair produce identical changes, which `Transaction::new`
collapses into one edit.

Character-operand commands are intercepted before the read-only check in
`execute_editor_command`, so the surround commands refuse read-only buffers
themselves.

Deviations from Helix: the pair `m r` and `m d` look for is one of the seven
`m a` understands (`(`, `[`, `{`, `<`, `"`, `'`, `` ` ``, with closing-bracket
aliases) or `m` for the closest, not any character. A count does not select
the nth enclosing pair.

Regression coverage, in `src/app/tests/surround.rs`:
`m_s_surrounds_the_selection_and_selects_the_pair_it_added`,
`m_s_wraps_every_selection_including_adjacent_ones`,
`surround_add_preserves_the_primary_when_inclusive_spans_merge`,
`m_r_replaces_the_named_or_closest_pair_around_the_cursor`,
`m_d_deletes_the_pair_around_the_cursor_or_the_pair_selected`,
`surround_edits_change_nothing_unless_every_cursor_has_a_pair`,
`escape_between_m_r_operands_cancels_the_replacement`,
`surround_pairs_resolve_through_the_syntax_tree`, and
`surround_edits_are_refused_in_a_read_only_buffer`. In `src/input_grammar.rs`:
`runyte_surround_replace_takes_two_character_operands` and
`vim_namespace_commands_collect_a_second_character_operand`. In
`src/command.rs`, `execution_context_resolves_character_operands_and_command_counts`
covers operand validation. `user_guide_action_reference_matches_the_registry`
in `src/keymap/actions.rs` checks the new action reference rows.

## Report

[Issue #14](https://github.com/runyte/runyte/issues/14) requests surround
editing: adding, replacing, and removing quotes or brackets around selections,
similar to Helix's `ms`, `mr`, and `md`. The report attaches a screenshot and
does not specify which characters count as pairs, how the pair is found, the
resulting selection, or multi-cursor behavior.
