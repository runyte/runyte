# Native window keeps waking and redrawing while nothing changes

The native window (`--window`, built with `--features native`) wakes its main
thread at the display refresh rate while idle, and re-renders the whole scene
on the GPU at that rate for one second after every input event, including
plain mouse motion.

## Measurements

Same environment as `native_window_frame_latency.md` (Linux, Radeon 880M,
rootful Xwayland at 60 Hz, 120×40 cells). Thread context switches and process
CPU time were sampled from `/proc`.

| Measurement | `--window` | Alacritty + `runyte --ide` |
| --- | ---: | ---: |
| Idle wakeups | 68–76 per second | 0 |
| CPU while typing about 7 keys per second | 4.4–6.5% | 0.9% |
| Wakeups while only moving the mouse over the window | about 460 per second | — |

## Diagnosis

GPUI 0.2.2 drives frames from a free-running source:

- X11: `start_refresh_loop` (`platform/linux/x11/client.rs`) is a calloop
  timer at the monitor refresh rate that runs for as long as the window is
  visible, whether or not anything changed.
- Wayland: `WaylandWindowStatePtr::frame` requests a new `wl_surface.frame`
  callback every time one fires, and `completed_frame` commits on every frame,
  so the callback loop never stops.

In the frame request handler in `window.rs`, a window that is not dirty is
still presented when its last input was less than one second ago:

```rust
// Keep presenting the current scene for 1 extra second since the
// last input to prevent the display from underclocking the refresh rate.
let needs_present = request_frame_options.require_presentation
    || needs_present.get()
    || (active.get()
        && last_input_timestamp.get().elapsed() < Duration::from_secs(1));
```

`Window::present` calls `PlatformWindow::draw` with the retained scene, which
renders the full scene on the GPU again. Each keystroke or pointer movement
therefore costs about 60 full-scene GPU renders over the following second.

## Expected behavior

On Linux an idle window has no periodic wakeups and renders nothing. A frame
is drawn and presented only when the window is dirty, when the platform
requires presentation (an X11 `Expose`, a Wayland configure or resize), or
when a next-frame callback or animation frame was requested. Input that
changes nothing on screen does not cause rendering.

## Constraints

- Shares the vendored GPUI patch with `native_window_frame_latency.md`; the
  wake that replaces the free-running source is the same one that allows
  immediate drawing.
- `Window::on_next_frame` and `request_animation_frame` keep working: an
  animation requested during a frame produces another frame one refresh
  interval later.
- A hidden or fully obscured X11 window still draws nothing, as today.
- macOS keeps its display-link behavior and the one-second presentation rule
  unless validated on a Mac.

## Reproduction

1. Run `target/release/runyte --window` on Linux and leave it idle.
2. Sample `voluntary_ctxt_switches` for the process's threads over a few
   seconds, or watch CPU while moving the mouse over the window without
   clicking.
