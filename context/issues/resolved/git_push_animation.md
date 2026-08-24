---
title: "Git background activity lacks clear status-row feedback"
status: resolved
reported: 2026-08-13
resolved: 2026-08-13
legacy_commit: 985fac2
---

## Resolution

Commit `985fac2` (`Add reusable long-running action animation`) resolves the
original issue. `App::git_summary` previously appended queued or running Git
work to the ordinary status text, but `draw_status` had no presentation for a
long-running action and the frontend event loops did not redraw merely because
time passed. The result was a static status row that could make a background
pull or push look as though nothing had happened.

The editor snapshot now carries a provider-neutral `LongRunningActionSnapshot`
with a label, target detail, elapsed time, and optional cancellation hint.
`draw_long_running_action` replaces the normal status row while that value is
present. Standalone and attached frontends advance the animation on an 80 ms
clock, and the local protocol carries the same immutable action snapshot. Git
mutations are the first producer through `App::long_running_action_snapshot`;
the rendering and wire types contain no Git-specific state, and the progress
surface stays in the shared status row rather than in Git buffers.

Commit `272d605` (`Use a rotating status spinner`) resolves the follow-up
report. `draw_long_running_action` was filling the remaining status width with
a highlighted segment that travelled along a horizontal track and bounced at
both edges. That made a simple activity signal occupy and animate many cells.
The renderer now chooses one frame from the exact `-`, `\`, `|`, `/` cycle,
reserves one cell for it, and pads the clipped action label so the spinner
stays at the right edge of the action area. Because the replacement is in the
shared renderer, Git push and every other producer of
`LongRunningActionSnapshot` use it in both standalone and attached frontends.
Unread notifications deliberately retain the absolute rightmost cells; when
they are present, the spinner sits immediately to their left instead of hiding
them.

Coverage lives in:

- `src/app.rs`: `git_mutations_feed_the_generic_long_running_action_snapshot`
- `src/ui.rs`: `long_running_action_uses_a_right_anchored_rotating_bar`
- `src/ui.rs`: `long_running_action_keeps_unread_notifications_visible`
- `src/protocol/mod.rs`: `long_running_action_snapshot_round_trips_over_the_wire`
- `src/app.rs`: `p_pulls_the_current_branch_and_shift_p_pushes_the_selected_one`

Known limitation: an attached TUI currently receives a complete immutable host
frame for each 80 ms animation step. A future frontend-local clock could reduce
serialization and channel traffic if more concurrent long-running producers
make that worthwhile.

## Report

The initial report stated that pressing `p` or `P` in either the Git status
buffer or Git branch buffer started a background action without showing any
animation that indicated it was being processed. The desired feedback was an
animation similar to `| -> / -> - -> \ -> |`, but with a more polished
appearance.

The follow-up report stated that the Git push and other action animation in the
status bar was too small and overcomplicated. It requested a simple rotating
bar with this exact sequence:

```text
-
\
|
/
-
```

The spinner was to occupy the right corner of the status bar, where the prior
animation ended on the right.
