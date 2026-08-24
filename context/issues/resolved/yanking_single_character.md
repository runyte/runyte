---
title: "y yanks the whole line instead of the character, and v y does nothing"
status: resolved
reported: 2026-08-13
resolved: 2026-08-13
legacy_commit: 41440a6
---

## Resolution

Commit `41440a6` (`Yank the caret character, and whole lines with Y`) fixed
both halves in `App::yank_value` and `App::yank`.

`yank_value` branched on whether any range was non-empty. The non-empty branch
took `selection_text`, which resolves each range through `operative_span` and
therefore already covers the character under a bare caret — the same span `d`
and `c` act on. The empty branch ignored that and built a linewise register
from the whole line each caret sat on, which is why `y` copied a line and why
its register pasted as a line. That branch is gone: yank now always reads
`selection_text`, so an empty range means one character and `y` agrees with
every other operator about what an empty range stands for.

`yank` returned Select mode to Normal only when a range was non-empty. After a
bare `v`, no range is, so `v y` wrote a linewise register, reported `yanked`,
and left the mode where it was — the reported "says yanked but nothing
happens". The mode is now handed back unconditionally in the extracted
`App::write_yanked_register`, which is the tail both yank commands share. The
selection itself is deliberately kept rather than collapsed, because `P` pastes
at its start and that behavior predates this issue.

The line yank `y` used to perform became `Y`, bound in the modal keymap as the
new `EditorCommand::YankLine`. It writes the register `x y` writes without
walking the selection through line mode: `App::line_register` takes every row
between `range.from()` and `range.to()` for each range, deduplicates, and
terminates each with a newline. Rows come from the raw range rather than from
`operative_span`, which for a caret on an empty last line resolves backwards
onto the preceding line's terminator and would yank the wrong row. Unlike `x`,
`Y` leaves the selection and the caret untouched: it is a copy, not a way of
choosing what to operate on next.

`Y` is not a Helix binding — Helix leaves it unbound — so it is recorded as an
addition rather than as compatibility, alongside `V`, in
`context/reference/helix-keymap-v1.md`.

One deliberate exception survives the change. In a directory buffer a bare
caret still yanks the whole entry on its row, linewise, because
`directory_register` already reads a caret there as naming the row's file
identity; yanking one character of a filename beside a register carrying that
file would make the two disagree.

Tests: `y_yanks_the_caret_character_and_capital_y_yanks_whole_lines` and
`explorer_yank_of_a_bare_caret_takes_the_whole_entry` in `src/app.rs`, which
also cover the two cases the row arithmetic above is shaped around — `Y` on a
caret sitting on an empty last line, and `Y` reaching a directory entry by the
path that does not run through `yank_value` — with
`x_and_x_yanks_paste_whole_lines_but_v_yanks_remain_characterwise` and
`visual_yank_includes_the_character_under_the_cursor` in the same file
covering the transient line selection and explicit `v` selections that had to
keep their existing behavior.

## Report

Yanking a single character was wrong in two ways. Pressing `y` on a character
copied the entire line. Pressing `v` then `y` entered Select mode and reported
that it had "yanked", but Select mode was not deactivated and the character was
not yanked; `v y` worked only when more than one character was selected.

The expected behavior:

```
y -> copy character under the cursor
v y -> copy character under the cursor (select mode should be deactivated after y)
Y -> copy entire line (same as x y)
```

These bindings were described as familiar to both Vim and Helix users.
