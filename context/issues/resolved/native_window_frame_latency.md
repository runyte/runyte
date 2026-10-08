---
title: "Native window frames wait for the next GPUI refresh tick"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: cd6f2fd
---

## Resolution

`cd6f2fd` (`Draw native window frames on demand on Linux`) builds GPUI 0.2.2
from a patched copy in `vendor/gpui`, wired through `[patch.crates-io]`.
`vendor/gpui/RUNYTE-PATCH.md` records provenance, the removed example programs
and every modified file; each modified file carries a notice, and
`THIRD_PARTY_NOTICES.md` records the Apache-2.0 copy.

The latency came from GPUI's frame source, not from Runyte's bridge. GPUI drew
only when the platform asked for a frame: X11's `start_refresh_loop` timer
fired at the display refresh rate, and Wayland drew only from `wl_surface.frame`
callbacks. `Window::refresh` and `cx.notify()` merely set the invalidator's
dirty flag, so a frame published by the host waited for the next tick.

`WindowInvalidator` now holds an optional platform wake, called when the window
goes from clean to dirty and when `Window::on_next_frame` registers a callback.
`PlatformWindow::frame_waker` supplies it; the default is `None`, so macOS, the
test platform and Windows keep their own frame sources. On X11 a per-window
scheduler replaces the periodic timer. A wake draws on the next event-loop
iteration through a `calloop` ping when no frame was drawn during the last
refresh interval, and one interval after the previous frame otherwise. A ping
is used rather than an immediate timer because calloop rounds timer waits up
to whole milliseconds. Wakes raised while a frame runs are paced from that
frame's start, so animations and self-renewing next-frame callbacks keep to the
refresh rate. On Wayland a dirty window is drawn at once unless a frame
callback is pending; callbacks are requested only for presented frames and for
frames during which another frame was requested. Nothing is drawn before the
first configure is acknowledged, and an interactive resize that sets GPUI's
resize throttle schedules the frame that clears it.

Removing the 60 Hz tick exposed two X11 behaviours the tick had hidden. The
Vulkan WSI waits for Present events from its own thread on the connection it
is given; on GPUI's connection those waits read key presses into libxcb's
queue without making the socket readable, and the event loop slept with input
pending for up to a second, or until other activity woke it. The surface now
uses a dedicated X connection. Events that GPUI's own reply waits queue are
drained after every event-loop iteration, as the tick used to do.

Measured with `tests/native_window.py --latency` at 120×40 cells (rootful
Xwayland at 60 Hz in a headless KWin, Radeon 880M): key-to-pixel median fell
from 12.6–15.9 ms to 4.6–5.2 ms, p90 from about 29 ms to 5.7–6.5 ms, and the
maximum from 33 ms to 7 ms. A probe that reads back only the start of the
edited row measured 3.9–4.5 ms, against 3.8 ms for the terminal frontend in
Alacritty. Idle context switches fell from 71 to 9 per second, which is part of
`native_window_idle_redraws.md`. The full figures are in
`context/reference/native-window-experiment.md`.

Coverage: `tests/native_window.py --latency` requires every one of 60 isolated
keystrokes to reach the screen within one second, which catches lost wakes and
stalled input, and with `--max-idle-wakeups` fails a window that returns to
periodic refresh; the native CI workflow runs it with a limit of 30 per second.
The registry build fails that limit at 69–73 per second. The standalone,
`--mux`, `--window-controls` and `--window-controls --mux` acceptance modes of
the same harness pass with the patched build. The harness now drops inherited
`RUNYTE_*` variables other than `RUNYTE_NATIVE_PAINT_TIMING`, so a run started
from a Runyte terminal no longer hands its parent context to the fixture.

Known limitation: the Wayland scheduler has not run on a desktop. The headless
compositor used for measurement offers no input injection and did not deliver
frame callbacks even to the unpatched build. macOS keeps GPUI's display-link
frames. The one-second re-presentation after input in GPUI's frame handler is
unchanged here; it is tracked by `native_window_idle_redraws.md`.

## Report

In the native window (`--window`, built with `--features native`), a typed
character reached the screen about three to four times later than the same
editor running in Alacritty, and the delay varied widely from key to key.

### Measurements

Release builds on Linux 7.2.8, AMD Radeon 880M (RADV), rootful Xwayland at
60 Hz inside a headless KWin virtual output. An XTest key press was sent
every 120–170 ms in Insert mode, and the edited cell was read back with
`XGetImage` until its pixels changed. The window was 1080×800 logical pixels
(120×40 cells) unless stated otherwise.

| Frontend | median | p10 | p90 | max |
| --- | ---: | ---: | ---: | ---: |
| `runyte --ide` in Alacritty | 3.8 ms | 2.9 | 4.6 | 5.1 |
| `runyte --window --ide` | 12.7–15.2 ms | 4.4–6.1 | 19–29 | 35 |
| `--window`, GPUI drawing as soon as the window is dirty | 3.4 ms | 2.7 | 4.4 | 5.4 |

At 212×57 cells the window measured 15.8 ms median (p90 31.4 ms) and 4.5 ms
(p90 5.4 ms) with immediate drawing; at 318×86 cells, 19.2 ms (p90 32.8 ms)
and 5.8 ms (p90 7.2 ms).

The last row used an experimental GPUI build whose X11 refresh timer polled
every millisecond and drew only when the window was dirty. It isolated the
cause and was not a usable fix, because polling cost about 1,000 wakeups per
second.

### Diagnosis

The keystroke path up to GPUI is short: GPUI's X11 event source handles the key
press as soon as the connection is readable, the bridge sends it to the host
thread, the host prepares and renders a frame (well under 1 ms at 120×40),
and the view task calls `cx.notify()` when the frame arrives.

GPUI 0.2.2 then drew only when the platform asked for a frame:

- X11 (`platform/linux/x11/client.rs`, `start_refresh_loop`) ran a timer at
  the monitor's refresh rate. A dirty window waited for the next tick, 0–16.7
  ms away at 60 Hz and occasionally a full extra tick.
- Wayland (`platform/linux/wayland/window.rs`, `frame`) drew only from
  `wl_surface.frame` callbacks and requested a new callback on every frame,
  so a dirty window waited for the compositor's next callback.
- macOS drives frames from a display link.

`Window::refresh` and `cx.notify()` only marked the window dirty; no public
GPUI 0.2.2 API asked the platform for an immediate frame. An X11 `Expose`
event only flagged that the next tick must present. Terminal emulators such
as Alacritty instead draw as soon as content changes when no frame was drawn
during the last refresh interval, and throttle only while frames arrive faster
than the display refreshes.

### Expected behavior

On Linux, a window that becomes dirty while idle is drawn without waiting for
a timer tick or frame callback. While frames arrive faster than the display
refreshes, drawing stays paced at the refresh interval. Key-to-pixel latency
matches or improves on the terminal frontend in a GPU terminal emulator.

### Constraints

- GPUI is pinned to `=0.2.2`. A change to it is carried as a documented local
  patch under `vendor/`, like `vendor/block` and `vendor/proc-macro-error2`,
  with its license and provenance recorded in `THIRD_PARTY_NOTICES.md`.
- macOS behavior stays unchanged unless validated on a Mac.
- Drawing stays on GPUI's main thread and outside any entity update; a
  platform wake must not re-enter the window while it is being updated.
- The bridge keeps its current semantics: latest frame only, coalesced
  wakeups, painted-frame identity and acknowledgements unchanged.
- Each edit publishes two host frames: one immediately and one when the
  background syntax parse returns. With immediate drawing both are painted
  (175 paints for 80 keys, against about 100 with the refresh tick). Both must
  render identically apart from refreshed highlighting.

### Reproduction

1. `cargo build --release --features native`.
2. Run `target/release/runyte --window` on X11 and open a source file.
3. Type in Insert mode while comparing against `target/release/runyte` in
   Alacritty, or measure as above.
