With soft wrapping enabled, a mouse wheel event over an inactive document pane
changes the active pane's viewport instead. `scroll_pane` calculates wrapped
rows from the requested pane, but mutates `active_mut()` in its wrapped branch.
The unwrapped branch correctly looks up the requested pane.

This can also panic on upward scrolling in debug builds: if the hovered pane
is on a continuation segment while the active pane's `scroll_wrap` is zero,
the branch subtracts one from the active pane's zero segment. The release build
wraps that subtraction instead of retaining a valid viewport.

The wheel should scroll only the pane under the pointer, preserve keyboard
focus and selections, and use that same pane's wrapping coordinates. Reproduce
with two document panes, soft wrapping enabled, a long line in the inactive
pane, and wheel events above that pane while the other pane remains active.
