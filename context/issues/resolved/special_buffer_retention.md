---
title: "Special buffers disappeared too early for adjacent history navigation"
status: resolved
reported: 2026-08-23
resolved: 2026-08-23
legacy_commit: 8c6bcec
---

## Resolution

Commit `8c6bcec` (`Retain two recent special buffers`) changed
`App::retire_detached_ephemeral_buffers`, whose previous candidate scan retired
every clean special buffer as soon as no pane displayed it. `retire_buffer`
then removed that buffer from each pane's jumplist, so `Ctrl-o` / `Ctrl-i` and
`Alt-o` / `Alt-i` had no adjacent special view to return to.

The editor now records live special-buffer recency and retains the two most
recently active clean special buffers. Activating a third retires the least
recent detached clean one, and explicit buffer retirement removes its identity
from the recency list. “Oldest” is deliberately interpreted as least recently
used: revisiting a special buffer through jump history refreshes its position.
Special buffers activated by asynchronous results are discovered before the
current buffer is marked recent, so an immediate history jump retains the true
activation order.
Empty clean scratch buffers retain their independent immediate-retirement
policy. The same lifecycle pass also preserves the last explorer directory
when a retained explorer becomes detached, so `:quit-here` does not depend on
the explorer being retired.

Coverage lives in `src/app/tests/git.rs` tests
`the_two_most_recent_clean_special_buffers_remain_jumpable`,
`opening_one_clean_special_buffer_past_the_limit_retires_the_least_recent_detached_one`,
`an_async_special_view_precedes_the_buffer_reached_by_immediate_history_navigation`,
and `an_empty_clean_scratch_buffer_retires_after_its_last_view_leaves`; in
`src/app/tests/editing_and_buffers.rs` test
`workspace_search_remains_jumpable_and_is_rebuilt_in_place`; and in
`tests/terminal.rs` test
`terminal_output_remains_jumpable_after_its_pane_returns_to_the_terminal`.
The retirement test was named for opening a third special buffer while the
retention limit was two; it was renamed and made limit-relative when the limit
was raised.

Known limitation: dirty special buffers are never discarded implicitly, and a
clean special buffer visible in a pane is never evicted out from under that
pane, so either condition can temporarily raise the live special-buffer count
above two.

## Report

Every special buffer was closed after switching to another buffer. This kept
too many buffers from accumulating in `Space b b`, but made it impossible to
switch back and forth between time-adjacent buffers with `Alt-i` / `Alt-o` and
`Ctrl-i` / `Ctrl-o`.

At most two special buffers were to remain in memory. Opening a third was to
close the oldest, preserving back-and-forth navigation between the adjacent
pair without allowing special buffers to accumulate.
