---
title: "Native window turns every fractional scroll event into a whole line"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: f19432e
---

## Resolution

`f19432e` (`Accumulate fractional native window scrolling into whole scroll
events`) replaces the per-event rounding in the `on_scroll_wheel` listener of
`NativeView::render` (`src/native_frontend.rs`) with a `ScrollAccumulator`
owned by the view.

The old listener converted each GPUI delta to lines and sent
`ceil(lines)`, at least one, crossterm scroll events. Two errors compounded.
Every fractional touchpad or high-resolution step became a whole event, and the
unit was wrong: the host scrolls three lines or columns per scroll event
(`src/app/input.rs`, the `ScrollUp | ScrollDown` arm), while GPUI 0.2.2 on
Linux reports one wheel notch as `Lines(3.0)` (`SCROLL_LINES`). One notch
therefore sent three events and moved nine lines, three times a terminal's
distance; the report assumed it sent one.

The accumulator works in host-event units. `ScrollAccumulator::delta` divides
`ScrollDelta::Lines` by the lines GPUI reports per notch (`WHEEL_LINES_PER_NOTCH`:
1 on macOS, whose non-precise wheels report about one line, and 3 elsewhere;
Windows scales by its own scroll-lines setting, 3 by default) and divides
pixel deltas by the cell size and the host's three lines per event. `push`
adds the delta to a per-axis remainder and returns the whole events now due,
keeping the fraction. The axis is chosen on the raw delta, because cells are
narrower than they are tall and converted horizontal motion would otherwise
outweigh vertical motion. A reversal discards that axis's remainder. GPUI on
Linux reports no touch phases, so a pause longer than 500 ms also ends the
gesture; `TouchPhase::Started`, reported on macOS, resets it too. A
tolerance of a thousandth of an event keeps sums such as thirty 0.1-line steps
from falling just short. The cap of twenty scroll events per GPUI event is
kept; anything beyond it is discarded rather than carried.

Media panes still handle their own pixel zoom and pan first. In persistent
attachments the emitted events pass through `PointerBatcher` as before, which
merges identical wheel events into repetitions.

Test: `fractional_scroll_accumulates_into_whole_host_events` in
`src/native_frontend/tests/input.rs` covers one event per notch, thirty 2 px
touchpad steps making one event, reversal, the gesture gap, the dominant axis
including a drifting vertical swipe, and the cap. The standalone and `--mux`
modes of `tests/native_window.py` pass, including their wheel-driven media
zoom.

Known limitation: GPUI does not forward Wayland's `axis_stop`, so on Linux a
gesture ends only after the pause. Horizontal motion within a mainly vertical
GPUI event is dropped rather than carried, as before. A Windows scroll-lines
setting other than three changes the speed proportionally.

## Report

In the native window (`--window`), touchpad scrolling and high-resolution
mouse wheels scrolled far too fast. A short touchpad swipe moved the buffer
many lines, and each small scroll event cost a full host frame.

### Diagnosis

The `on_scroll_wheel` listener in `NativeView::render`
(`src/native_frontend.rs`) converted each GPUI `ScrollWheelEvent` on its own:

```rust
let delta = event.delta.pixel_delta(px(metrics.height));
// ...
f32::from(delta.y).abs() / metrics.height,
// ...
for _ in 0..(amount.ceil() as usize).clamp(1, 20) {
    view.send(mouse_event(/* ScrollUp or ScrollDown */));
}
```

`amount.ceil()` with a lower bound of one turned any non-zero delta into at
least one whole line. GPUI 0.2.2 reports:

- X11: `ScrollDelta::Lines` with fractional values from XInput 2 smooth
  scrolling, for touchpads and high-resolution wheels;
- Wayland: `ScrollDelta::Pixels` with continuous pixel values for touchpads,
  and `ScrollDelta::Lines` for discrete wheel steps;
- macOS: `ScrollDelta::Pixels`, including momentum events.

A swipe that the platform reported as 30 events of 0.1 lines therefore sent 30
scroll events to the host instead of 3, each a full scroll step followed by a
full frame. The horizontal axis had the same problem.

### Expected behavior

Fractional deltas accumulate per axis, and one scroll event is sent to the
host for each whole line; the remainder carries over to the next event. A
reversal of direction discards the remainder. One discrete wheel notch still
sends exactly one scroll event, as today. Scrolling over media panes keeps its
existing pixel zoom and pan behavior.

The report's "as today" for wheel notches was mistaken; see the resolution.

### Constraints

- Applies in standalone windows and persistent-session attachments; both
  receive the same crossterm mouse events from the bridge.
- The existing cap of 20 lines per event stays.
- Each emitted line keeps the pointer position and modifiers of the event that
  completed it.

### Reproduction

1. Run `target/release/runyte --window` on a laptop and open a long file.
2. Scroll with a two-finger touchpad swipe and compare the distance with the
   same gesture in a terminal frontend.
