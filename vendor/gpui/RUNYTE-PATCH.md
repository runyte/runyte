# Local patch

Source: crates.io `gpui` 0.2.2 by Zed Industries, Inc., Apache-2.0
(`LICENSE-APACHE`, unchanged). Upstream repository:
<https://github.com/zed-industries/zed>. The published archive is copied
as-is except for the changes listed here; `Cargo.toml.orig` and
`.cargo_vcs_info.json` are retained as provenance. Each modified source file
carries a notice at its top.

To audit the copy, compare it with the registry archive:

```sh
diff -r ~/.cargo/registry/src/*/gpui-0.2.2 vendor/gpui
```

The expected differences are the files below, this document, the removed
`examples/` directory and the removed `.cargo-ok` registry marker.

## Removed

- `examples/` (4.8 MB) and the thirty `[[example]]` entries in `Cargo.toml`.
  They are GPUI's own demonstration programs and are not built by Runyte.

## On-demand frames on Linux

GPUI 0.2.2 draws only when the platform asks for a frame: X11 runs a timer at
the display refresh rate for every visible window, and Wayland requests a new
`wl_surface.frame` callback on every frame. A window made dirty by input
therefore waited up to a full refresh interval before it was drawn, and an
idle window kept waking at the refresh rate.

- `src/window.rs`: `WindowInvalidator` can hold a platform wake. It is called
  when the window becomes dirty (`set_dirty(true)` or `invalidate_view` from a
  clean state) and when `Window::on_next_frame` registers a callback. The wake
  only schedules work on the platform event loop; drawing never happens
  re-entrantly inside an entity update.
- `src/platform.rs`: `PlatformWindow::frame_waker` returns that wake. The
  default is `None`, so macOS, Windows and the test platform keep their own
  frame sources unchanged.
- `src/platform/linux/x11/{client,window}.rs`: the periodic refresh timer is
  replaced by a per-window scheduler. A dirty window is drawn on the next
  event-loop iteration (through a `calloop` ping) when no frame was drawn
  during the last refresh interval, and one interval after the previous drawn
  frame otherwise. A wake raised while a frame runs (an animation, a
  next-frame callback, an effect flushed after drawing) is paced from that
  frame's start, so frames never exceed the refresh rate and a self-renewing
  next-frame callback cannot spin. Visibility still gates drawing; a window
  that becomes visible is drawn and presented once. `Expose` schedules a
  presenting frame.
- `src/platform/linux/x11/client.rs`: X events that libxcb queued while GPUI
  waited for a reply are drained after every event-loop iteration. The
  periodic refresh used to drain them on each tick.
- `src/platform/linux/x11/window.rs`: the Vulkan surface uses a dedicated X
  connection. The WSI waits for Present events on the connection it is given,
  from its own thread; on GPUI's connection those waits read input events into
  libxcb's queue without making the socket readable, so the event loop slept
  with key presses pending until unrelated activity woke it.
- `src/platform/linux/wayland/{client,window}.rs`: a dirty window is drawn on
  the next event-loop iteration unless a frame callback is pending, in which
  case the callback draws it. Nothing is drawn before the first configure is
  acknowledged; that configure draws the window, as upstream. A frame callback
  is requested for a frame that is presented, and for a frame during which
  another frame was requested, so such requests wait for the compositor. An
  interactive resize that sets GPUI's resize throttle schedules a frame,
  because only a frame clears the throttle. Every later configure schedules a
  frame as well: GPUI commits the surface when a frame completes, even when
  nothing was drawn, and that commit applies the acknowledgement and window
  geometry of a configure that changed no size.

## Presentation without changes

- `src/window.rs`: GPUI presents the retained scene again on every frame
  request for one second after any input, to keep variable-refresh displays
  from lowering their rate. `Window::present` re-renders the whole scene on the
  GPU. On platforms that draw on demand a frame is requested only when
  something changed, so the rule is skipped there; platforms with a periodic
  frame source keep it.

## Build hygiene

- `src/taffy.rs`: two float literals are written as `f32`. The compiler already
  inferred `f32` through a fallback that is being phased out; behaviour is
  unchanged. As a path dependency, GPUI's warnings are no longer capped, and
  the native CI build requires a warning-free build.

Remove this patch when upstream GPUI draws on demand on Linux, or replace it
with a fork if further GPUI changes accumulate.

## Native cell scene reuse

- `src/window.rs`: `SceneCache` and `Window::paint_cached_scene` expose a
  draw-only subset of GPUI's existing previous-scene replay. Runyte uses it for
  unchanged cell rows, avoiding repeated raster-bound and sprite-atlas lookups.
  Each handle contains only indices into the immediately preceding window frame;
  no extra scene, glyph layout, or texture is retained. Window identity, frame
  generation, scale, clipping, opacity and element offset must match. The caller
  also verifies row content and placement. Replay recalculates draw order using
  the current scene, preserving original layer operations. Input handlers,
  element states and layout registrations must not be placed inside this cache.
  The normal input handler and frame acknowledgement remain outside it.

The scheduling patch and the draw-only replay API are independent. Removing the
Linux scheduling changes after an upstream update must retain the replay API
until Runyte's cell painter has an equivalent upstream interface.
