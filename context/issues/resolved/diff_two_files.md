---
title: "No way to compare two files side by side"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: 6767736
---

## Resolution

Fixed by 6767736 "Compare two buffers side by side".

The report asked how to build this rather than only for the behaviour, and
the answer it was looking for — do not invent a view for diffing — turned out
to be available almost entirely from parts that already existed. Splits,
per-pane scroll, and directional focus were already there, so the two-pane
half of the request needed nothing. What was missing was an alignment, a way
for a pane to show a row belonging to no line, and a link between two panes'
scroll positions.

`src/git/diff.rs` already ran a longest-common-subsequence alignment of two
texts, but its only public function was `changed_rows`, which projected the
result onto one side and threw the correspondence away. That correspondence
is exactly what a side-by-side view needs, so the algorithm moved to a new
provider-neutral `src/diff.rs` that returns an `Alignment`: runs of `Equal`,
`Replaced`, `Inserted`, and `Deleted` lines carrying both sides' row ranges.
`git::changed_rows` is now a reading of that alignment rather than a second
implementation of it, which is what keeps the Git gutter and this view from
ever disagreeing about what changed. The move is behaviour-preserving; every
pre-existing test in `src/git/diff.rs` passes unmodified.

An alignment also defines an **aligned row space**: a run occupies as many
rows as its longer side has lines, and the shorter side leaves the remainder
empty. That space is the whole mechanism. Two panes handed the same aligned
starting row independently project their own lines where they have them and
filler where they do not, and end up level with each other without either
pane knowing the other exists. The scroll link is therefore one integer on
the session rather than a pane-to-pane coupling.

`src/diff_view.rs` holds that session: the two panes and buffers, the cached
alignment, the buffer revisions it was built from, and the shared aligned
start. It is deliberately expressed in buffer ids rather than paths, so a
later Git comparison can supply a base revision as a read-only buffer and
reuse the whole mechanism. The report asked for the design to anticipate that
and this is the entirety of what it required.

Filler goes through `project_visible_rows`, the existing single projection,
rather than a second one — the same function folds already deviate from
document rows in. `PreparedRow::document_row` became `Option<usize>` so that
a row belonging to no line is a type rather than an invariant a future caller
could forget: the compiler then required every consumer to say what filler
means to it. A click resolves to nothing, `goto-word` offers no label, and
`H`/`M`/`L` skip it, all matching how the blank area past the last line
already behaves.

Scroll synchronisation settles after the panes are projected rather than
before. The leading pane's scroll row is not final until its own caret has
been accounted for, so deriving the shared aligned start beforehand left the
following pane one frame behind; `settle_diff_scroll` re-projects both sides
from the position they agreed on.

Two deviations from what the report described are worth naming. The command
is `diff-this`, not `difft`: the hyphenated form matches `git-diff` and the
rest of the inventory, and `difft` and `dt` are registered aliases. They had
to be aliases rather than abbreviations because `parse_colon_command`
resolves a name exactly — prefix matching drives only Tab completion — so
`:difft` would otherwise have failed on Enter despite completing. `:diff-off`
takes the alias `do`, which is an exact match and so is unambiguous against
`document-outline` and `document-symbols`.

The second is that the buffer marked first is placed on the left. When the
marked buffer is not on screen the second command splits for it, and because
a split always appends its new pane after the one it came from, the marked
buffer takes over the original pane while the buffer being compared moves
into the new one. This reads in the order the two commands were typed. Left
and right are otherwise read off the layout tree, whose walk order is its
screen order, so two buffers already in panes keep the panes they are in and
the side coloured as added is always the one on the right.

Soft wrap is forced off in a comparison and restored when it closes, and
collapsed regions are expanded when one opens. Both follow from lines being
matched whole: a wrapped line takes a different number of screen rows on each
side, and a hidden line cannot sit level with anything.

Changed lines take a whole-line background from three new optional theme
keys, with values added to all five bundled themes. They are optional and
resolve to `None` rather than to a terminal colour, unlike the existing
`change_*` keys, because a fill covering a whole line has to be a tint of the
background; a theme predating this keeps working and shows the gutter bar
alone. In a comparison that gutter column shows the comparison rather than
the Git marks, since one column cannot answer both questions.

Tests: `src/diff.rs` covers the run structure, the aligned row space, the
round trip between a row and its aligned position, and the bounded fallback
for regions too large to align. `src/diff_view.rs` covers which rows face
each other across a gap and when an alignment is recomputed. In `src/app.rs`,
`marking_two_buffers_opens_them_side_by_side`,
`buffers_already_in_two_panes_keep_them`,
`a_line_only_one_side_has_holds_the_other_side_open`,
`each_side_is_coloured_by_what_it_has`,
`scrolling_one_side_moves_the_other_to_the_line_facing_it`,
`editing_a_side_realigns_the_view`, `a_diff_pane_does_not_soft_wrap`,
`marking_the_same_buffer_again_takes_the_mark_back`,
`diff_off_closes_the_comparison_and_keeps_both_panes`,
`closing_a_pane_ends_the_comparison`,
`showing_another_buffer_ends_the_comparison`,
`the_comparison_commands_answer_to_their_short_spellings`, and
`what_cannot_be_compared_says_so` cover the command surface, the projection,
the scroll link, and teardown.

Known limitation: differences are shown per line, not within a line. A
changed line is filled whole rather than highlighting the spans that actually
differ, which a word-level refinement of each `Replaced` run would add later
without changing anything below it. Comparison is also confined to two
buffers already open in the editor; there is no command yet that compares a
file against a Git revision, though the session was shaped so that such a
command needs only to supply a second buffer.

## Report

A feature was requested for diffing two selected files, activated by opening
a file buffer and typing `:difft`, then activating another file buffer — in
the same pane or another, which should not matter — and typing `:difft`
again, at which point a diff view appears.

The view should be a two-pane layout with one file on the left and the other
on the right, preferably synchronised when scrolling. Nothing like it existed
in the editor at the time.

The design was to be consistent with the rest of the editor rather than
custom-built for diffing alone, and was to anticipate the same view being
reused for Git diffs later. How to implement it was left open for discussion.

The short spellings `:dt` for `:difft` and `:do` for `:diff-off` were
requested separately while the work was under way.
