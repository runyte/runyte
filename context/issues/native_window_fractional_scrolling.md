# Native window turns every fractional scroll event into a whole line

In the native window (`--window`), touchpad scrolling and high-resolution
mouse wheels scroll far too fast. A short touchpad swipe moves the buffer
many lines, and each small scroll event costs a full host frame.

## Diagnosis

The `on_scroll_wheel` listener in `NativeView::render`
(`src/native_frontend.rs`) converts each GPUI `ScrollWheelEvent` on its own:

```rust
let delta = event.delta.pixel_delta(px(metrics.height));
// ...
f32::from(delta.y).abs() / metrics.height,
// ...
for _ in 0..(amount.ceil() as usize).clamp(1, 20) {
    view.send(mouse_event(/* ScrollUp or ScrollDown */));
}
```

`amount.ceil()` with a lower bound of one turns any non-zero delta into at
least one whole line. GPUI 0.2.2 reports:

- X11: `ScrollDelta::Lines` with fractional values from XInput 2 smooth
  scrolling, for touchpads and high-resolution wheels;
- Wayland: `ScrollDelta::Pixels` with continuous pixel values for touchpads,
  and `ScrollDelta::Lines` for discrete wheel steps;
- macOS: `ScrollDelta::Pixels`, including momentum events.

A swipe that the platform reports as 30 events of 0.1 lines therefore sends
30 scroll events to the host instead of 3, each a full scroll step followed
by a full frame. The horizontal axis has the same problem.

## Expected behavior

Fractional deltas accumulate per axis, and one scroll event is sent to the
host for each whole line; the remainder carries over to the next event. A
reversal of direction discards the remainder. One discrete wheel notch still
sends exactly one scroll event, as today. Scrolling over media panes keeps
its existing pixel zoom and pan behavior.

## Constraints

- Applies in standalone windows and persistent-session attachments; both
  receive the same crossterm mouse events from the bridge.
- The existing cap of 20 lines per event stays.
- Each emitted line keeps the pointer position and modifiers of the event that
  completed it.

## Reproduction

1. Run `target/release/runyte --window` on a laptop and open a long file.
2. Scroll with a two-finger touchpad swipe and compare the distance with the
   same gesture in a terminal frontend.
