---
title: "Periodic refresh discarded an unfinished command and every search match"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: 4d4dd46
---

## Resolution

Commit `4d4dd46` (`Defer periodic Git refresh while a view is being worked
in`) resolved this report.

Both symptoms had one cause. `App::replace_virtual_preserving_row` rebuilds a
refreshed projection's selection from a single row identity per pane and
installs it with `Selection::point`, so any multi-range selection is discarded;
a search that has selected every match is exactly such a selection. The
unfinished query was lost for a related reason: it lives in command mode, which
the refresh had no reason to consult. `App::request_periodic_git_refresh`
declined only while a Git mutation was in flight, so the five-second timer fired
straight through both situations.

The report offered three options: re-search after each refresh, keep the old
matches, or skip the refresh while a command is being typed or text is
selected. The third was implemented, as suggested. Re-searching would have to
invent a policy for matches that the refreshed text no longer contains, and
keeping old matches would leave selections pointing at offsets whose content
had changed — both would present stale or invented state as if it were current,
which is worse than briefly not refreshing.

`request_periodic_git_refresh` now also declines while a prompt is open, which
is how `/`, `s`, and `S` take their query, and while a projection buffer holds a
deliberate selection. Deliberate reuses the rule `App::scoping_region` already
applies when deciding whether a search narrows: a bare caret is a one-character
range in this grammar, so several ranges, or one range of two or more
characters, distinguish a selection the person made from a cursor position.

The deferral is restricted to the buffers a refresh actually rewrites —
`is_refreshed_projection` covers the status, branches, worktrees, log, blame,
stash, and diff views. A selection in a tracked source file does not defer
anything, because a refresh reconciles that buffer's gutter rather than
replacing its text; treating it as at risk would have stalled the timer for as
long as any selection existed anywhere.

Nothing is dropped by waiting. `WorkspaceHost::refresh_git_if_due` records its
timestamp only when the request is accepted, so a skipped tick is retried on the
next one and the refresh lands as soon as the prompt closes or the selection
collapses. `:git-refresh` continues to reconcile immediately.

The report's closing note that this "probably affects all buffers which are
periodically refreshed" was correct, and the fix is written at that level rather
than for the log view alone.

A follow-up report noted that a refresh also moved the cursor to the first
column, and asked whether the pause could extend to a timeout after the last
action rather than only to prompts and selections. Both were addressed.

`App::replace_virtual_preserving_row` carried only the row across a refresh and
rebuilt the selection with `Selection::point(line_to_offset(row))`, which put
the cursor at the start of the line. It now carries the column as well and
clamps it to the length of whatever row it lands on, so a shorter replacement
row leaves the cursor at its end rather than past it.

`App` also records `last_interaction`, stamped in `handle_input` and
`handle_pointer`, and `interaction_defers_git_refresh` waits out one refresh
interval after it. The interval is reused rather than configured separately:
it already expresses how much staleness is acceptable, and a second knob for
the same judgement would be one more thing to explain. The practical effect is
that the automatic refresh only lands while the person is idle, which is what
the report asked for.

Tests: `periodic_refresh_defers_to_an_open_prompt_and_to_search_matches`,
`periodic_refresh_ignores_a_selection_outside_a_git_projection`,
`periodic_refresh_waits_out_the_interval_after_the_last_keystroke`, and
`refreshing_a_projection_keeps_the_cursor_column`, all in `src/app.rs`.

Known limitation: an explicit `:git-refresh`, and the mandatory reconciliation
that follows a Git mutation, still rebuild the selection from one row identity
and so still collapse search matches down to the single cursor, though that
cursor now keeps its column. Only the unattended periodic refresh defers. A
refresh the person asked for, or one that follows a change they made, is
reporting something they need to see, so suppressing it would be wrong; making
those preserve multi-range selections would require mapping every range through
the new text and is a separate change.

## Report

In the Git log view (`Space g l`), searching for a string with `/`, `s`, or `S`
loses work when the buffer is refreshed:

- any unfinished command being typed, such as a search query, is discarded;
- all search matches disappear.

The buffer's content changes across a refresh, so new matches may appear. The
possible approaches are to re-search after each refresh, to keep only the old
matches, or to skip refreshing while a command is being typed or while text is
selected.

This probably affects all buffers which are periodically refreshed, though only
the log view was checked.
