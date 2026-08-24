---
title: "Command mode looked identical to Normal mode and had no colour of its own"
status: resolved
reported: 2026-08-23
resolved: 2026-08-23
legacy_commit: 1d36db0
---

## Resolution

Commit `1d36db0` (`Distinguish command mode and terminal review`) gave command
mode both halves of what it was missing: a colour of its own, and a visible
sign that the keyboard has left the panes.

`Theme` gained a fourth mode role, `cursor_command`. `TuiTheme::cursor` had
been collapsing `Mode::Command` onto `cursor_normal`, so a prompt looked
exactly like Normal mode at both the caret and the status label; it now
answers with the new role, and `text_run_style` was rewritten to call
`TuiTheme::cursor` instead of repeating the mode match inline, so the caret,
the terminal caret, and the status label cannot disagree about what a mode
looks like. Every bundled theme names a purple explicitly. `light` and `paper`
use the purple already in their own `keyword`/`attribute` colours (`#8250df`
and `#8700af`), palette-driven families use the palette's own purple —
Catppuccin's `mauve`, Everforest's `purple`, Nord's `#b48ead` — and the
nineteen Zenbones palettes, which have no purple upstream, share one dark and
one light Runyte purple the way their jump labels already do. A custom theme
that omits the role falls back to `info`, the one semantic colour the other
three modes had not already claimed, so a compact theme still separates four
modes.

Everforest's light `purple` is a pale magenta that fails text contrast behind
a caret glyph painted in the background (2.96 against `#fffbef`). Those themes
therefore keep upstream's purple for syntax and use a darker `#bf4d9a` for the
caret, which is the same accommodation the file already makes for jump labels.

The dim reuses the mechanism `goto-word` already had rather than adding a
second one. `PaneSnapshot::jump_active` had been doing two jobs: deciding
whether a pane carries jump labels, and deciding whether its text is muted.
The second job moved to a new `PaneSnapshot::dimmed`, and `snapshot_line` and
`terminal_line` now read that instead. Because the terminal branch of
`snapshot_pane` already muted child cells through the same flag, terminal
panes were covered without a second code path. `dimmed` is set from
`App::command_prompt_dims_panes`, which reads only `self.mode`, so it is true
for every pane rather than only the active one, and a pending `g` or `Space` —
which lives in `RunyteGrammar::pending` and is not a mode — cannot reach it.

Two deliberate deviations from the report. Every prompt that enters
`Mode::Command` dims, not only `:`: `self.mode = Mode::Command` is assigned in
exactly one place, `App::open_prompt`, and searches, rename, and the rest all
route through it, so dimming for the mode rather than for one prompt kind
needs no list that a future prompt could be left off. And the caret is not
dimmed: it keeps the colour that names the mode, since dimming the one thing
that says which mode this is would work against the rest of the change.
`goto-word` still dims its caret with everything else, where the labels are
what has to stand forward. The prompt and its completion list needed no work —
both are drawn outside `draw_pane`.

A terminal under review is dimmed on the same flag. Review is the frozen half
of a terminal pane, where the child has stopped painting and the keys move a
caret over a still image, and nothing on screen said so except the title.
Moving to another pane does not begin review, so a live terminal left behind
keeps its colours; `i` discards review and restores them.

`editor.command_mode_dim` turns the prompt dim off, defaulting on. The review
dim is not covered by it: that one reports what a terminal is rather than
decorating a mode.

Both new fields cross the bundled local protocol, so `protocol::VERSION` moved
from 26 to 27 and an older attached client is refused at the handshake rather
than drawing undimmed panes.

Covered by `config::tests::built_in_themes_use_mode_specific_cursor_colors`,
`config::tests::every_built_in_command_cursor_is_legible_against_its_own_ground`,
and the hue and contrast assertions in
`config::tests::built_in_search_selection_palettes_are_legible_and_role_distinct`,
all in `src/config.rs`; `snapshot::tests::a_command_prompt_dims_every_pane_and_a_pending_chord_dims_none`,
`snapshot::tests::a_search_prompt_dims_the_panes_too`, and
`snapshot::tests::the_command_dim_can_be_turned_off` in `src/snapshot.rs`;
`ui::tests::a_command_prompt_dims_the_text_in_every_pane`,
`ui::tests::the_command_prompt_and_its_caret_are_not_dimmed`,
`ui::tests::a_dimmed_terminal_grays_the_childs_colours_but_not_its_caret`,
`ui::tests::editor_caret_uses_the_theme_color_for_each_mode`,
`ui::tests::active_terminal_cursor_uses_the_theme_color_for_each_mode`, and
`ui::tests::only_the_status_mode_label_follows_the_caret_color` in `src/ui.rs`;
and `a_terminal_is_dimmed_while_it_is_under_review_and_colourful_while_it_is_live`
plus `moving_away_from_a_live_terminal_does_not_dim_it` in `tests/terminal.rs`.

Known limitation: a terminal child that paints its own cell backgrounds keeps
them while dimmed. Only the foreground is muted, for the same reason the
inactive-pane ground applies only where the child delegated its background —
rewriting a child's palette would make its TUI semantically wrong. Such a pane
reads as dimmer but not as fully grayed.

## Report

Command mode was expected to have an optional visual effect of graying out all
text in all panes:

- when a user presses `:`, all text in all panes is grayed out
- the "gray" colour does not necessarily need to be exactly gray; the colour
  should be adapted to the theme being used
- if the current themes already have a class of syntax effect for a "gray out"
  effect, that colour should be used; if not, a new one should be added
- the grayed out effect should stay until the user exits command mode, which
  can happen after pressing Esc, Ctrl-c, or Enter
- the command tooltip should not be grayed out
- other motions like `g`, `Space`, and so on should not trigger it

The effect was also expected to include terminal panes.

The same graying was expected for a terminal's `[review]` mode, and only for
review: the standard Normal mode that leaves the terminal active, such as after
moving to another pane with `Ctrl-h/j/k/l`, was expected to keep the terminal
colourful. Review is the mode that leaves the terminal frozen and allows the
cursor to move around, and it was expected to be clearly marked by grayed out
terminal text. Going back to Insert or Normal mode was expected to make the
terminal colourful again.

In addition, a new colour was expected for command mode, used for the cursor
and for the part of the status bar which shows the coloured mode: NOR, SEL,
INS, CMD.

A matching colour could be chosen for each theme, with some variant of purple
suggested for most themes. For the `paper` and `light` themes exactly purple
was wanted, so that the modes are distinguished:

    INS - red
    NOR - blue
    SEL - orange
    CMD - purple
