# Native window silently drops input when its queue is full

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
