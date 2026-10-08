---
title: "Native window host renders a frame for every queued input event"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 2d85e47
---

## Resolution

`2d85e47` (Batch queued native keys before publishing a bounded frame).

The standalone native loop previously prepared a complete frame after every
queued key or text event, even though the GUI would display only its latest
frame. `Events::defer_frame` now checks for already waiting key/text input
without consuming its captured attachment or painted-frame envelope. It drains
at most 64 applied events before publication and never waits for another event
to arrive. A lone event still renders immediately; acknowledgements do not defer
it. The existing key-repeat detector continues to observe each event.

Deferral sets the shared `frame_pending` flag before the next event is admitted.
Approval checks therefore reject input against unpublished changes even when the
host has not prepared a new frame id yet. Ordinary queued editing remains
accepted. The terminal frontend and persistent-session host retain their own
loops. Pointer, resize and focus events are publication barriers, preserving
geometry-sensitive admission and lifecycle handling; adjacent redundant drags
are already coalesced by the native input queue instead of being batched across
potential layout changes.

`queued_native_keys_publish_bounded_batches_without_reordering_text` in
`src/native_frontend/tests/input.rs` applies an Insert-mode sequence through the
real workspace host and verifies exact text and publications after events 64,
128 and 129. `native_batching_never_waits_for_acknowledgements_or_crosses_pointer_and_resize`
in that file covers lone input, acknowledgements and ordering barriers.
`native_batch_pending_blocks_approval_even_before_frame_identity_changes` covers
a queued Enter against an unpublished filesystem confirmation whose frame id
still matches. Existing native approval and attachment tests remain passing.
`tests/native_window.py --input-burst` injects 512 characters without per-key
sleeps and checks the saved document preserves their order.

## Report

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
