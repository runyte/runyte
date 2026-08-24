---
title: "`e` opens entries in the explorer, and no mode explains its own keys"
status: resolved
reported: 2026-07-31
resolved: 2026-07-31
legacy_commit: 055503a
---

## Resolution

Fixed by 055503a "Give every view a `:?` help window and free `e` in the
explorer".

**Request 1.** `src/keymap.rs` registered two directory-scoped bindings for
`open-directory-entry`, Enter and `e`. Directory-scoped bindings shadow global
ones, so `e` in a directory buffer never reached `move-word-end`. The `e` row
is gone; Enter is the only key that opens an entry, and a directory buffer now
has the same word motions as any other buffer. Nothing else changed: `-` and
Backspace still open the parent, `r` still refreshes, and the `Ctrl-w v/s`
split-open bindings are untouched. The task-list picker's `e` was deliberately
left alone — that is a modal overlay, not a buffer, so no motion competes with
it there.

**Request 2.** `draw_help` in `src/ui.rs` rendered registry bindings and
nothing else, which described available keys but not the active view. The new
`src/help.rs` owns the missing half: a `HelpTopic` chosen from the mode and the
binding scope, each carrying hand-written prose. Scope wins over mode, so an
explorer stays the explorer topic while its listing is being edited. NORMAL
doubles as the program's main help, SELECT explains selection extension, and
EXPLORER explains that the listing is an ordinary buffer, that Enter opens the
entry on the line, and that edits reach disk only through a confirmed plan.
The module is presentation-neutral; `ui` decides how to draw a title and a list
of lines.

`?` became an alias of the existing `help` command, so `:?` opens that window
from every view that can reach the command palette. `Space ?` was already
bound and still works.

Two things had to change for the topic to be truthful. The palette assigns
`Mode::Normal` before it runs a command, so a topic derived at render time
would answer NORMAL for `:?` typed from SELECT. `open_prompt` now records the
mode the palette was opened from, and the help window captures its topic when
it opens rather than deriving it later; `show_help: bool` became
`help: Option<HelpTopic>` so the flag and the topic cannot disagree.
All help entry points go through `open_prompt`, which keeps that recorded
origin stable.

The help window's height budget was inverted to match: the overview claims
rows first and the generated key table takes what is left, with the separator
and footer appearing only once there is a table to introduce. A window too
short for both keeps the orientation and drops the key list.

Tests: `e_is_the_word_end_motion_inside_directory_buffers` and
`space_splits_are_symmetric` in `src/keymap.rs`;
`e_moves_by_word_inside_a_directory_buffer`,
`question_mark_command_opens_help_for_the_current_view`,
`help_from_select_mode_describes_select_mode`, and
`help_follows_the_most_recent_prompt_origin` in `src/app.rs`; the topic
selection tests in `src/help.rs`; and
`help_opens_on_an_overview_naming_the_current_view` plus
`help_keeps_its_overview_when_the_terminal_is_tiny` in `tests/key_hints.rs`.

Known limitation: below roughly six rows of editor area the popup is all
border with no interior, so nothing is drawn inside it. The key table is still
ordered by registry declaration order rather than by usefulness, so the first
screenful is mostly motions; the prose carries the commands worth knowing
instead.

## Report

Two requests.

First, opening a file in the explorer used either `e` or Enter. Since
directories are treated as normal buffers, `e` conflicts with the word-end
motion, so only Enter should open a file.

Second, a uniform way to discover the available keybindings in the current
mode, triggered by the same keys everywhere — `:?<Enter>` — opening a popup
naming the current mode and how to navigate it. In the explorer it would state
that the view is navigable and editable like a normal buffer, that Enter and
`-` move between directories, and that Enter opens files. In Normal mode the
same popup would carry the main help for the program: what it is, how to use
it, and how to reach further help.
