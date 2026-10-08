# Native window frames wait for the next GPUI refresh tick

In the native window (`--window`, built with `--features native`), a typed
character reaches the screen about three to four times later than the same
editor running in Alacritty, and the delay varies widely from key to key.

## Measurements

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
every millisecond and drew only when the window was dirty. It isolates the
cause; it is not a usable fix, because polling costs about 1,000 wakeups per
second.

## Diagnosis

The keystroke path up to GPUI is short: GPUI's X11 event source handles the key
press as soon as the connection is readable, the bridge sends it to the host
thread, the host prepares and renders a frame (well under 1 ms at 120×40),
and the view task calls `cx.notify()` when the frame arrives.

GPUI 0.2.2 then draws only when the platform asks for a frame:

- X11 (`platform/linux/x11/client.rs`, `start_refresh_loop`) runs a timer at
  the monitor's refresh rate. A dirty window waits for the next tick, which is
  0–16.7 ms away at 60 Hz and occasionally a full extra tick.
- Wayland (`platform/linux/wayland/window.rs`, `frame`) draws only from
  `wl_surface.frame` callbacks and requests a new callback on every frame, so
  a dirty window waits for the compositor's next callback.
- macOS drives frames from a display link.

`Window::refresh` and `cx.notify()` only mark the window dirty; no public GPUI
0.2.2 API asks the platform for an immediate frame. An X11 `Expose` event only
flags that the next tick must present. Terminal emulators such as Alacritty
instead draw as soon as content changes when no frame was drawn during the
last refresh interval, and throttle only while frames arrive faster than the
display refreshes.

## Expected behavior

On Linux, a window that becomes dirty while idle is drawn without waiting for
a timer tick or frame callback. While frames arrive faster than the display
refreshes, drawing stays paced at the refresh interval. Key-to-pixel latency
matches or improves on the terminal frontend in a GPU terminal emulator.

## Constraints

- GPUI is pinned to `=0.2.2`. A change to it is carried as a documented local
  patch under `vendor/`, like `vendor/block` and `vendor/proc-macro-error2`,
  with its license and provenance recorded in `THIRD_PARTY_NOTICES.md`.
- macOS behavior stays unchanged unless validated on a Mac.
- Drawing stays on GPUI's main thread and outside any entity update; a
  platform wake must not re-enter the window while it is being updated.
- The bridge keeps its current semantics: latest frame only, coalesced
  wakeups, painted-frame identity and acknowledgements unchanged.
- Each edit currently publishes two host frames: one immediately and one when
  the background syntax parse returns. With immediate drawing both are painted
  (175 paints for 80 keys, against about 100 with the refresh tick). Both must
  render identically apart from refreshed highlighting.

## Reproduction

1. `cargo build --release --features native`.
2. Run `target/release/runyte --window` on X11 and open a source file.
3. Type in Insert mode while comparing against `target/release/runyte` in
   Alacritty, or measure as above.
