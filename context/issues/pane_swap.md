# Swapping the active pane with the previously focused one

There is no way to exchange two panes. `Space w` and `Ctrl-w` hold focus,
splitting, closing, only-window, equalizing, full-screen, and zen; a reader who
wants the file they are editing on the other side of the split has to close a
pane and open it again, losing that pane's selection and scroll position.

## Expected behavior

A new binding `Space w x`, with the compatibility alias `Ctrl-w x`, exchanges:

- the pane the caret is in, and
- the pane the caret was in before it.

Vim spells the same action `Ctrl-w x`, which is why the alias takes that key.

The previously focused pane is already derivable: `App::activate_pane`
(`src/app/file_workflows.rs`) records a monotonic order per pane in
`pane_activated_at`, and `pane_focus_rank` reads it to break ties when
choosing a directional neighbour. The pane to swap with is the highest-ranked
pane that is not the active one.

## What moves

The two panes exchange their **contents** — buffer or terminal, selection,
scroll position, buffer history — rather than their positions in the split
tree, which is what Vim's `Ctrl-w x` does. The layout is left alone, so the
boundary between the two panes does not move and a pane that spanned the
editor still does. The two readings are indistinguishable when the panes have
the same shape and differ when they do not.

The caret stays with the content it was in, so the reader is still editing
what they were editing, now in the other position.

## Points the fix has to settle

- **The inverse.** After the swap, the previously focused pane record must be
  such that pressing the sequence again undoes it rather than picking a third
  pane.
- **Nothing to swap with.** With one pane, or with no recorded previous pane,
  or when the previous pane has since been closed, the command reports a
  refusal on the interaction line rather than doing nothing silently.
- **A maximized view.** `:zen` and `:fullscreen` already make `focus` and
  `next-window` refuse with a stated message, because the maximized pane is
  the only one keys can reach. This command refuses the same way.
- **Terminals.** A terminal is pane content rather than a buffer, so it is one
  of the things a swap moves. The exchange must carry terminal ownership and
  review state with it, settle the destination the way `finish_pane_focus` and
  `settle_terminal_focus` do, and must not restart the child; the PTY sees a
  resize only when the pane it lands in has a different size.
- **Modes.** `Space w x` is Normal and Select, as the other `Space w`
  bindings are. Decide whether `Ctrl-w x` joins the restricted `Ctrl-w` set
  available from Insert and Terminal Insert, which today admits `h/j/k/l`,
  `w`, `v`, `s`, `f`, and `z`. Following `Ctrl-w w` is the consistent answer.

## Constraints

- Registered in `src/keymap.rs` with the other window bindings, so help and
  key hints describe it without a second table. `Space w x` is Primary and
  `Ctrl-w x` Compatibility, matching how the rest of the namespace is
  classified.
- `context/reference/helix-keymap-v1.md` gains a row in the Window and Space
  modes section, stating that Vim spells it the same way and what the fix
  decided about content versus position.
- `docs/user-guide.md` (Files and splits) and the window help text describe
  it.

## Regression coverage

Cover in `tests/keymap.rs` and `tests/maximized_panes.rs`: the swap exchanges
two panes and leaves the caret with its content; a second press is the inverse
of the first; a single pane and a closed previous pane are refused with a
message; a maximized view refuses; and a terminal pane keeps its session and
its review state across the swap.
