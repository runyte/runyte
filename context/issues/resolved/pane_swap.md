---
title: "The active pane cannot exchange contents with the previously focused pane"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 2b1719e
---

## Resolution

Commit 2b1719e (`Add pane content swapping`) added the `swap-window` command.
`App::swap_window` was added beside the existing pane-focus workflows. Its
first implementation selected the highest-ranked surviving pane, which meant
closing the immediate predecessor silently exposed an older pane as the swap
target. Pane activation now retains the immediately preceding pane identity
separately from the ranked activation history used by directional focus. The
command uses that exact identity and refuses if it has been closed. It
exchanges the complete `Pane` values while leaving the split tree and pane
geometry untouched, then activates the position holding the original content.
Activating that destination makes the former position the immediate
predecessor, so invoking the command again is its inverse.

Content-owned state kept outside `Pane` moves with it: pristine search
selection presentation, side-by-side diff ownership, and tutorial pane roles
are remapped between the two pane identities. Terminal ownership is part of
the exchanged `Pane`; focus settlement therefore preserves the same terminal
session and its live or review state while normal frame preparation resizes it
only if its destination geometry differs.

The command refuses with an interaction-line error when there is no immediate
predecessor or that pane has closed, and it uses the existing maximized-view
reason for both Zen and full-screen refusal. `Space w x` is the Primary binding
in Normal and Select modes. Vim's `Ctrl-w x` spelling is retained as a
Compatibility binding and joins `Ctrl-w w` in Insert, Replace, and Terminal
Insert. Unlike Vim, the command exchanges pane contents rather than pane
positions.

Regression coverage:

- `pane_swap_bindings_have_primary_and_compatibility_roles_in_every_window_mode`,
  `pane_swap_exchanges_complete_contents_follows_the_caret_and_is_its_own_inverse`,
  and `pane_swap_refuses_when_there_is_no_previous_pane` in `tests/keymap.rs`;
- `a_maximized_pane_refuses_content_swapping_in_both_views` in
  `tests/maximized_panes.rs`;
- `pane_swap_moves_a_terminal_session_and_preserves_its_review` in
  `tests/terminal.rs`;
- `swapping_pane_ownership_keeps_each_side_attached_to_its_buffer` in
  `src/diff_view.rs`;
- `pane_swap_refuses_when_the_immediately_previous_pane_was_closed`,
  `pane_swap_moves_both_diff_sides_with_their_buffers`,
  `pane_swap_moves_pristine_search_presentation_with_its_content`, and
  `pane_swap_moves_tutorial_roles_with_their_buffers` in `src/app/tests/`;
- `terminal_insert_swap_keeps_the_live_child_and_resizes_at_its_new_geometry`
  in `tests/terminal.rs`.

## Report

There was no way to exchange two panes. `Space w` and `Ctrl-w` held focus,
splitting, closing, only-window, equalizing, full-screen, and Zen commands, so
moving the file being edited to the other side of a split required closing and
reopening a pane and lost its selection and scroll position.

The expected primary binding was `Space w x`, with Vim's `Ctrl-w x` as a
compatibility alias. It was to exchange the pane containing the caret with the
previously focused pane, derived from the monotonic activation order recorded
by `App::activate_pane` in `pane_activated_at`. The candidate was the
highest-ranked pane other than the active one.

The two panes needed to exchange their contents — buffer or terminal,
selection, scroll position, and buffer history — rather than their positions
in the split tree. The layout and its boundaries were to remain fixed, while
the caret followed its original content to the other position. A second
invocation needed to undo the first instead of selecting a third pane.

With one pane, no recorded previous pane, or when the previous pane had since
been closed, the command needed to report a refusal on the interaction line.
Zen and full-screen presentations needed to refuse for the same reason as
focus and next-window: the maximized pane is the only pane reachable by keys.

A terminal needed to move as pane content without restarting its child. Its
ownership and review state needed to survive, destination focus needed the
same settlement used by ordinary pane focus, and its PTY was to receive a
resize only when the destination pane had a different size.

`Space w x` needed to work in Normal and Select modes. `Ctrl-w x` needed to
join the restricted `Ctrl-w` set available from Insert and Terminal Insert,
following the precedent of `Ctrl-w w`. Both bindings needed to live in the
shared keymap registry so dispatch, help, and key hints remained aligned. The
user guide, window help, and Helix keymap register needed to state the
content-versus-position decision and the Vim provenance of the alias.
