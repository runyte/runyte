---
title: "A failed search prompt's error never reached the interaction line"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: f13ebdd
---

## Resolution

Commit `f13ebdd` (`Echo a failed search prompt on the interaction line`)
found and closed the gap: the interaction line's action echo is written by
`report_completed_action`, into `App::action_feedback`, and only
`report_completed_action` writes it. `App::snapshot` shows that field
(`displayed_status_message()`) whenever the editor is not actively showing a
prompt or a live pending keystroke; nothing else feeds it.

Every command dispatched through `handle_editor_input`'s grammar loop
already wraps its result with `CommandState::capture`/`outcome` and calls
`report_completed_action`, so a failed `n`, `N`, `*`, or `#` — which reuse
`step_search`/`find_search` directly from a key binding — already echoed
correctly. The three "Runyte" search flavours (`s`, `S`, `/`) and the Vim
grammar's directional `/` and `?` take their pattern through a prompt
instead, and `handle_search_prompt`'s `KeyCode::Enter` arm called
`commit_search`/`find_search` straight from the key handler, without ever
going through that mechanism. `commit_search`/`find_search` called
`self.error(...)` on no match, which only set `self.status`/`status_error`
and pushed a retained notification — never `action_feedback` — so the
message existed only in `:notifications`, exactly as reported.

The fix wraps that one unwrapped arm the same way: the shared tail of the
`KeyCode::Enter` match (reached only by `PromptKind::Search`,
`SearchForward`, and `SearchBackward` — every other prompt kind is matched
above it) now takes a `CommandState::capture(self)` before running
`commit_search`/`find_search`, and calls `report_completed_action` with a
spelling (`s`, `S`, `/`, or `?`) and description matched from `kind`
afterward. This is the same shape `handle_command`'s colon-command arm and
`run_context_action` already use, not a new mechanism — it closes the one
remaining prompt-driven case the existing report
(`error_text_in_the_interaction_line.md`) didn't reach, since that fix
worked from `CommandOutcome` values a wrapped dispatch produces, and this
path bypassed the wrapper entirely. The other prompt kinds handled in the
same function (`Rename`, `GlobalSearch`, `FilterSelections`, worktree and
branch prompts, `JoinDelimiter`) were not touched: the report was about
search specifically, and each of those already calls its own `self.error`
without evidence of the same bypass being a problem worth guessing at
blind.

**"Pattern not found" is now a warning.** A new `search_warning` method,
next to `error`/`error_from`, mirrors `error` but retains its notification
at `NotificationSeverity::Warning` with source `"Runyte"` and title
`"Search"`, and — critically — leaves `status_error` `false`. All three
`self.error(format!("pattern not found: {}", ...))` sites (`commit_search`,
`step_search`, `find_search`) now call `search_warning` instead. Because
`CommandState::outcome`'s `UserError` branch is gated on `app.status_error`,
this alone reclassifies the outcome from `CommandOutcome::UserError` to
`CommandOutcome::Status`, which `report_completed_action` echoes without the
interaction line's error color — so the styling fix falls out of the
severity fix rather than needing separate handling. An invalid regular
expression, from either the prompt or `n`/`N`, is unaffected and still
echoes as a genuine error.

Current behavior further refines that retained severity from `WARNING` to
`INFO`: a search that ran successfully and found no match is an empty result,
not a condition requiring attention. The producer is now `search_info`; its
non-error interaction-line styling is unchanged.

Tests, in `src/app.rs`:

- `a_failed_search_keeps_the_previous_one_working` (existing, updated) now
  asserts `!app.status_error` for a failed `s` search, and additionally pins
  the exact `displayed_status_message()` text and
  `!displayed_status_message_is_error()`.
- `an_invalid_regex_from_the_search_prompt_echoes_as_an_error` drives an
  invalid pattern through the `/` prompt and checks both `status_error` and
  that the echoed interaction-line text names the failure.
- `a_vim_search_prompt_with_no_match_echoes_as_information` drives the Vim
  grammar's `/` through a `vim_app` fixture and checks the same non-error
  echo for that grammar's prompt kind.

Known limitation: the fix covers the three search prompt kinds named in the
report. Whether the same "call `self.error`, but nothing reads it back"
shape exists in the other prompts `handle_search_prompt` also drives
(`Rename`, `GlobalSearch`, `FilterSelections`, and the Git worktree/branch
prompts) was not investigated end to end and is left for a future report if
one of them turns out to have the same problem.

## Report

Searching for a missing string in `lorem_ipsum.md` produced an error:

```text
s xxx Enter
pattern not found: xxx
```

However, this error is not displayed in the interaction line. The
interaction line should display such errors as well. There is already a
mechanism for that, but it was not applied to all actions.

`pattern not found` should be a warning rather than an error.
