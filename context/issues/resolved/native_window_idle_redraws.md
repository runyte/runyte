---
title: "Native window keeps waking and redrawing while nothing changes"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: c413623
---

## Resolution

`c413623` (`Present native window frames only when content changes`) completes
work begun by `cd6f2fd` (`Draw native window frames on demand on Linux`,
recorded in `native_window_frame_latency.md`). That commit replaced GPUI's
free-running X11 refresh timer and perpetual Wayland frame-callback loop with
on-demand frames, which removed the periodic idle wakeups.

What remained was the frame-request handler in `vendor/gpui/src/window.rs`. It
presented the retained scene again whenever a frame was requested within one
second of input, and `Window::present` re-renders the whole scene on the GPU.
The rule exists so that variable-refresh displays do not lower their rate. It
now applies only where `WindowInvalidator::draws_on_demand()` is false, so macOS
and other platforms with a periodic frame source keep it. A Linux window
presents only what changed; required presentation (X11 `Expose`, a newly
visible window, the first frame, resizes and scale changes) still goes through
`require_presentation` or a dirty window.

The rule also used to keep a Wayland frame-callback loop running for a second
after input, and that loop committed acknowledgements of configures that change
no size: maximizing or tiling at the same size, inset or decoration changes.
Every configure after the first now schedules a frame; GPUI commits the surface
when a frame completes, even when nothing is drawn, which applies the
acknowledgement and window geometry.

The report's comparison of typing CPU against Alacritty was wrong: the probe
sampled only the Alacritty process, while the editor runs in its own session
under it. Measured per thread on the same 1,200-line Rust file, the window and
the terminal frontend spend about the same per keystroke on the syntax worker
(3.2–3.5 ms) and the host loop (3.0–3.2 ms). The window's GPUI main thread adds
2.0 ms, which takes the place of the terminal emulator's own rendering. Neither
frontend's per-keystroke cost is idle overhead.

Measured after the change (rootful Xwayland at 60 Hz, 120×40 cells):

| Measurement | Before `cd6f2fd` | After |
| --- | ---: | ---: |
| Idle context switches, whole process | 68–76 per second | 8–9 per second |
| Idle wakeups of the GPUI main thread | about 60 per second | 0 |
| Pointer motion over an idle window | about 468 per second | about 127 per second, nothing rendered |

The remaining idle wakeups come from the host's maintenance timers, file and
Git monitors and tokio workers; the terminal frontend shows about 7 per second
from the same sources. A tokio blocking thread present only in the window adds
2 per second.

Coverage: `tests/native_window.py --latency --max-idle-wakeups 30`, run by the
native CI workflow, fails the registry build at 69–73 per second and passes
with this change at 9. The standalone and `--mux` acceptance modes pass. The
per-thread figures are recorded in `context/reference/desktop-edition.md`.

Known limitation: the Wayland changes have not run on a desktop (see
`native_window_frame_latency.md`). If a Vulkan swapchain becomes out of date
without a size change, Blade skips that frame and nothing requests another
until the content changes again; the periodic re-presentation used to hide
this. A resize, which is the usual cause, repaints as before.

## Report

The native window (`--window`, built with `--features native`) woke its main
thread at the display refresh rate while idle, and re-rendered the whole scene
on the GPU at that rate for one second after every input event, including plain
mouse motion.

### Measurements

Same environment as `native_window_frame_latency.md` (Linux, Radeon 880M,
rootful Xwayland at 60 Hz, 120×40 cells). Thread context switches and process
CPU time were sampled from `/proc`.

| Measurement | `--window` | Alacritty + `runyte --ide` |
| --- | ---: | ---: |
| Idle wakeups | 68–76 per second | 0 |
| CPU while typing about 7 keys per second | 4.4–6.5% | 0.9% |
| Wakeups while only moving the mouse over the window | about 460 per second | — |

The Alacritty column counted only the Alacritty process; see the resolution
for the corrected typing comparison.

### Diagnosis

GPUI 0.2.2 drove frames from a free-running source:

- X11: `start_refresh_loop` (`platform/linux/x11/client.rs`) was a calloop
  timer at the monitor refresh rate that ran for as long as the window was
  visible, whether or not anything changed.
- Wayland: `WaylandWindowStatePtr::frame` requested a new `wl_surface.frame`
  callback every time one fired, and `completed_frame` committed on every frame,
  so the callback loop never stopped.

In the frame request handler in `window.rs`, a window that was not dirty was
still presented when its last input was less than one second ago:

```rust
// Keep presenting the current scene for 1 extra second since the
// last input to prevent the display from underclocking the refresh rate.
let needs_present = request_frame_options.require_presentation
    || needs_present.get()
    || (active.get()
        && last_input_timestamp.get().elapsed() < Duration::from_secs(1));
```

`Window::present` called `PlatformWindow::draw` with the retained scene, which
rendered the full scene on the GPU again. Each keystroke or pointer movement
therefore cost about 60 full-scene GPU renders over the following second.

### Expected behavior

On Linux an idle window has no periodic wakeups and renders nothing. A frame is
drawn and presented only when the window is dirty, when the platform requires
presentation (an X11 `Expose`, a Wayland configure or resize), or when a
next-frame callback or animation frame was requested. Input that changes
nothing on screen does not cause rendering.

### Constraints

- Shares the vendored GPUI patch with `native_window_frame_latency.md`; the wake
  that replaces the free-running source is the same one that allows immediate
  drawing.
- `Window::on_next_frame` and `request_animation_frame` keep working: an
  animation requested during a frame produces another frame one refresh
  interval later.
- A hidden or fully obscured X11 window still draws nothing, as before.
- macOS keeps its display-link behavior and the one-second presentation rule
  unless validated on a Mac.

### Reproduction

1. Run `target/release/runyte --window` on Linux and leave it idle.
2. Sample `voluntary_ctxt_switches` for the process's threads over a few
   seconds, or watch CPU while moving the mouse over the window without
   clicking.
