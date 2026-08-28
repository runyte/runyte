---
title: "The interaction line names a failure without saying why"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 86ae080
---

## Resolution

Commit `86ae080` (`Carry a failed or unavailable action's message onto the
interaction line`) changed what `report_completed_action` and
`mark_action_feedback_failed` write into the echo when a command fails or is
unavailable, and moved truncation to where the line's actual width is known.

**The shape.** Both functions already had the message in hand — it is
exactly what `error_from`/`mark_unavailable` had just pushed to the
notification center as `app.status` — but discarded it, composing only
`"{description} · failed — see :not"` or `"{description} · unavailable —
see :not"`. They now append it through a shared `outcome_clause(outcome,
message)` helper in `src/app.rs`, as `"{description} · failed: {message}"`
or `"{description} · unavailable: {message}"`. The `see :not` pointer is
dropped rather than kept alongside the message: it existed only because the
reason was missing, `:not` puts a colon straight after a colon command's own
leading one when a spelling like `:lsp-status` is involved, and `:not`
already remains where the untruncated text is read regardless — that last
fact is now stated once in `README.md` instead of on every echo. The
`mark_action_feedback_failed` guard that used to match on the literal
`"failed — see :not"` substring to avoid double-appending now checks
`ActionFeedback::is_error` instead (see below), which does not depend on the
suffix text at all.

**Truncation moved to the render pass.** The report asks for truncation
against "the space left on the line", and the first version of this fix
truncated at composition time against a fixed cell budget instead, reasoning
that the echo is composed nowhere near a render pass. That reasoning was
sound but the conclusion was backwards: `outcome_clause` now writes the
message in full, untruncated, and `draw_status` in `src/ui.rs` — which
already receives `interaction_line_area` and therefore the real number of
cells available this frame — truncates the composed line through a new
`clip_interaction_line(text, width)`. It cuts a multiline echo to its first
line, then to whatever cells remain, and appends a trailing `...` (three
literal dots, matching the report's own wording, not the `…` glyph
`clip_with_ellipsis` uses for the compact status-row labels a few hundred
lines above it) whenever either cut removed anything. Cells are counted with
`UnicodeWidthStr`/graphemes, the same convention `clip_with_ellipsis` and
`clip_path_start` already use in that file. Because an echo always has the
`spelling (detail)` shape `report_completed_action` composes, a closing `)`
that would otherwise fall on a dropped second line is kept immediately after
the marker instead of silently disappearing with whatever followed it.

This fixes a second, pre-existing bug as a side effect: before this change,
a long *description* alone (no message involved at all) simply ran past a
narrow terminal and was hard-clipped by Ratatui with no `...`, since nothing
upstream of the `Paragraph` widget ever measured the line's width. That case
now gets the same treatment as a long message.

One thing does not follow the frame's width: an active prompt (the command
palette, a search query) also lives in `interaction_line`, and its cursor
column is computed separately, against the *untruncated* string
(`prompt_cursor_column` in `src/snapshot.rs`). Clipping the text a prompt is
still being typed into would desynchronize the visible text from that
column, so `draw_status` only clips when `status.prompt_cursor_column` is
`None` — which, by construction, is exactly when the line is an action echo
rather than live input.

**Error styling.** `ActionFeedback` gained an `is_error: bool`, set directly
from the `CommandOutcome` that produced it (`true` only for `UserError`,
including the asynchronous Git-mutation-failure path through
`mark_action_feedback_failed`; `false` for `Unavailable`, `Completed`,
`Status`, and `AsynchronousRequest`). `StatusSnapshot.interaction_line_error`
was hardcoded `false` in `App::snapshot`; it now reads
`App::displayed_status_message_is_error()`, a small `pub(crate)` accessor
next to `displayed_status_message`, whenever the line is showing an echo
rather than a prompt. `draw_status` already branched on this field to color
the line with `theme.error`, so this was a real, if narrow, existing gap:
before this change, a failed action's echo was never actually drawn in the
error color. It now is, and an unavailable action or a success is not,
which is the "same error/non-error styling distinction" the notification
counts on the status line above it already make by severity.

**Coverage of "warnings and informational messages".** The report names
these alongside "a failure" and "an unavailable action" as outcomes that
should gain their message. Investigating every `NotificationSeverity::
Warning` and `info_from` producer in the codebase (there is exactly one
`Warning` producer, `mark_unavailable`, and two `info_from` call sites)
found:

- Every `mark_unavailable` call reached through a synchronous command
  dispatch — which is all of them except one, below — already flows through
  `outcome()`'s `self.unavailable_revision != app.unavailable_revision`
  check into `CommandOutcome::Unavailable` regardless of which command
  triggered it, whether or not that command is one of the few
  `CommandOutcomeHint::Unavailable`-hinted ones. So the fix above already
  covers essentially all of "warnings ... currently only retained": pressing
  `|` (`ShellPipe`, statically unsupported), asking for a document outline
  or syntax fold this buffer's syntax does not support, and every other
  `mark_unavailable` site now echoes its reason inline.
- The one exception is the `command_capabilities().command_availability()`
  check in `handle_command`'s `KeyCode::Enter` arm, which runs while the
  command palette is still open and returns before `execute()` — and
  therefore before `report_completed_action` — ever runs. Echoing there
  would mean closing the palette out from under whoever is still typing,
  which is precisely the case `README.md`'s "notifications never replace
  the prompt or action echo" rule exists to prevent; the comment left at
  that call site names this file. It stays retained-only, exactly as
  before, and `unavailable_colon_command_stays_typed_and_leaves_the_prior_
  echo_alone` in `src/app.rs` pins that this is deliberate rather than an
  oversight.
- `info_from("Git", "Git operation completed", summary)` in
  `apply_git_mutation_result` already updates the live echo first, via
  `update_action_feedback`, whenever there is one to update; it only falls
  back to retained-only when that action has already been superseded by a
  newer one (no live echo to attach to) or the summary is multiline (the
  first line already went to the echo through the same call, and the
  overflow is what gets retained). Neither is the gap the report describes.
- `info_from("LSP", "Language server status", message)` and the paired
  `error_from` in `apply_lsp_event`'s `LspEvent::Status` arm carry no
  correlation back to the action, if any, that triggered them —
  `LspEvent::Status` is `{ message, error }` with no request or action id,
  unlike `LspEvent::Response { token, .. }` or Git's `GitRequestId` ↔
  `git_action_origins` mapping. Threading one through would mean extending
  the LSP event pipeline itself, well past the shape of this fix, so these
  remain retained-only.
- `report_host_error`'s doc comment already states its own reasoning:
  "Hosts enter here so it is retained without replacing the action echo."
  Its callers are host-boundary failures (a startup-timing write, an RPC
  dispatch) with no specific triggering key, so this is the same "no action
  in flight" case and was left alone.

Tests, all updated or added for the final shape:

- In `src/app.rs`: `failed_action_echoes_its_message_inline_in_full` and
  `unavailable_action_echoes_its_message_inline_and_is_not_styled_as_an_
  error` call `report_completed_action` directly and check both the
  composed (untruncated) text and `displayed_status_message_is_error`.
  `unsupported_key_binding_echoes_its_message_inline` drives the same
  outcome through a real key press (`|`). `unavailable_colon_command_stays_
  typed_and_leaves_the_prior_echo_alone` pins the one deliberate exception
  above. `asynchronous_git_mutation_failure_echoes_its_message_inline`
  (existing, updated) drives `mark_action_feedback_failed` through a real
  `GitServiceEvent::Completed` failure. `counted_colon_binding_echoes_
  failure_and_retains_its_info_notification` (existing, updated) covers a
  counted binding's grammar-level rejection.
- In `src/snapshot.rs`: `a_failed_binding_marks_the_interaction_line_as_an_
  error_but_an_unavailable_one_does_not` drives a real key press through
  each outcome and checks `StatusSnapshot.interaction_line_error`.
- In `src/ui.rs`: `interaction_line_echo_is_clipped_to_the_frames_width`
  renders a known-length message at a narrow width and checks the exact
  truncated cells, then re-renders it at a generous width and checks it is
  returned whole. `interaction_line_echo_keeps_only_its_first_line_when_
  clipped` checks the multiline cut and the preserved closing paren.
  `an_active_prompt_is_never_clipped_with_a_marker` checks that a live
  prompt past the frame's width is never given a `...` marker.

Known limitation: an echo that fits within the frame's width entirely on its
own axis can still, in principle, contain a description long enough that
little room is left for the message before the line's actual width is
reached; that is exactly the width-aware case this fix now handles
correctly rather than a separate limitation.

## Report

When an action fails, the interaction line says only that it failed and
where to look. Pressing `p` in a read-only buffer echoes:

```
p (Paste after the selection · failed — see :not)
```

The message explaining *why* it failed exists — it is the text that went to
the notification center — but reading it costs a trip to `:not` and back.
For the common case, where the reason is one short sentence, that is a
detour for information that would have fit on the line that is already
being drawn.

The interaction line should carry the failure's own message, not just the
fact of the failure. When the message is longer than the space left on the
line, it should be truncated with a trailing `...`, and `:not` remains
where the full text lives.

This applies to the same set of outcomes that produce the current text: a
failure, an unavailable action, and the warnings and informational messages
that are currently only retained. The echo keeps its existing shape — the
key or command spelling, then the action's description — and gains the
message after it.

How much of the line the message may take, and what happens when the action
description itself is already long, is part of the work rather than
something the report settles.
