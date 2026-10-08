# Native window host renders a frame for every queued input event

The standalone host loop in `src/main.rs` handles one input event per loop
iteration and prepares and renders a complete frame after each one. The
native window (`--window`) only ever displays the latest frame the bridge
holds, so when several input events are already queued (key repeat, fast
typing during a slow frame, a burst of drag events) the host renders frames
that are replaced before GPUI can paint them, and the later events wait
behind that rendering work.

## Measurements

Per-event host cost before the GUI thread is involved, release build, plain
text buffer (median): 0.14–0.37 ms to prepare a frame and 0.19–1.10 ms to
render it through ratatui, for grids from 120×40 to 426×108 cells, plus the
grid copy described in `native_window_frame_copy.md`. A burst of N queued
events therefore delays the last event's frame by N times that cost.

No user-visible backlog has been measured with ordinary typing; the cost
matters for key repeat on large grids and for slower frames, such as those
with many diagnostics or large terminal panes.

## Expected behavior

When more native input events are already waiting in the bridge queue, the
host applies them before rendering, and renders one frame for the result.
Rendering after an event that arrived alone is unchanged.

## Constraints

- Approval admission must not change. `Events::accepts_input` compares the
  frame painted when the physical input was captured with the host's current
  frame and refuses while publication is pending. An input applied after an
  earlier input in the same batch must see publication as pending, because its
  effect has not been prepared or painted.
- Key-repeat detection (`KeyRepeatDetector`), presentation acknowledgements,
  resize and close events keep their current handling and ordering.
- The terminal frontend's crossterm event path is unchanged unless the same
  reasoning is verified for it separately.
- The number of events applied before a frame is bounded, so a continuous
  input stream cannot starve rendering.

## Reproduction

Hold a motion key (for example `j`) in a large window with
`RUNYTE_NATIVE_PAINT_TIMING=1` and compare the number of host frames with
the number of painted frames.
