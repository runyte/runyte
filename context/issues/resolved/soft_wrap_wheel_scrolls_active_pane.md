---
title: "Soft-wrap wheel scrolling changes the active pane instead of the hovered pane"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 6bcf501
---

## Resolution

Commit `6bcf501` (`fix(panes): scroll the hovered pane when soft wrapping`).

`App::scroll_pane` derived wrapping coordinates from the requested pane but
mutated `active_mut()`. Looking up `pane_id` for that mutation keeps geometry
and state attached to the same pane, also preventing the mismatched upward
scroll subtraction from underflowing.

Coverage: `soft_wrap_wheel_scrolls_the_hovered_inactive_pane_in_both_directions`
in `src/app/tests/presentation_and_settings.rs` sends physical pointer events
over an inactive pane and verifies both scroll directions, unchanged keyboard
focus, and preserved selections. It failed before the fix and passes after it.

## Report

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
