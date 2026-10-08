---
title: "Native window silently drops input when its queue is full"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: b09ceca
---

## Resolution

`b09ceca` (Bound native input separately from paint acknowledgements and report overflow).

`Bridge::send` previously shared a bounded Tokio channel with presentation
acknowledgements. Every queued drag occupied a slot, and rejection was reported
only to standard error. The new native `input_queue` keeps physical input in a
bounded FIFO and the latest acknowledgement in a separate slot. Adjacent drags
coalesce only when attachment, captured painted frame, button and modifiers
match; a key, paste, click, release or geometry event remains an ordering barrier.
Each retained physical event keeps its original painted-frame identity.

The physical queue holds at most 4,096 events and 8 MiB of pasted text. The GUI
never waits for the host to consume input: rejected input sets a sticky flag and
wakes the frontend independently of host progress. The window displays “Some
input was lost. Click to dismiss.” Successful subsequent input does not clear
the notice. The dismissal click is consumed by the notice. A Tokio notification
wakes the receiver without polling. Physical input is drained before the latest
acknowledgement; acknowledgements cannot occupy physical-input capacity or grant
an input a newer painted-frame identity.

`acknowledgements_do_not_displace_physical_input_and_overflow_stays_visible`,
`drags_coalesce_only_across_matching_identity_button_and_modifiers`,
`queued_paste_bytes_are_bounded_and_released_on_receive`, and
`queue_wakes_without_polling_and_drains_before_sender_close` in
`src/native_frontend/tests/input_queue.rs` cover capacity, captured identities,
10,000 drag events, ordering barriers, bounded paste storage, retained rejection,
notification and shutdown. Existing admission/attachment tests in
`src/native_frontend/tests/input.rs` run through the new queue.

Known limitation: a stalled host can still exhaust the bounded queue with
non-coalescible physical input. Those events are refused visibly rather than
buffered without limit; the notice cannot reconstruct lost input.

## Report

`Bridge::send` in `src/native_frontend.rs` forwards each physical input event
to the host with `try_send` on a bounded channel of 4,096 entries. When the
channel is full, the event is discarded and only a line is written to
standard error:

```text
Runyte native input queue is full
```

A window is usually started from a desktop launcher, so standard error is
not visible, and the user sees no indication that keystrokes were lost.

The queue fills when the host stops reading for a while (for example during a
long synchronous operation) while input keeps arriving. Pointer drags fill it
fastest: every `MouseMoveEvent` with the left button pressed becomes a
separate `Drag` event, and high-rate pointing devices report hundreds of
motion events per second. Presentation acknowledgements share the same
channel.

## Expected behavior

- Keys, text, clicks and paste are never discarded silently. If an event has
  to be refused, the window shows that input was lost; it does not merely
  write to standard error.
- Consecutive pointer drag events that the host has not consumed yet are
  coalesced into the latest position, so motion alone cannot fill the queue.
- Presentation acknowledgements cannot displace physical input.

## Constraints

- GPUI's main thread must never block on the host.
- Memory stays bounded while the host is stalled.
- The order of physical input events is preserved, and each keeps the
  painted-frame identity captured when it arrived.

## Reproduction

Make the host loop stall (for example, a debugger breakpoint in the host
thread), drag with the left mouse button for several seconds, then type.
Release the host and observe missing characters with the message only on
standard error.
