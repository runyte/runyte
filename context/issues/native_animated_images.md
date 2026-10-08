# Animated GIF and WebP images show only their first frame in the native window

In `runyte --window`, GIF and WebP paths open as read-only `[image]` media
projections, like PNG, JPEG and BMP. An animated GIF or animated WebP shows
only its first frame, as a still image. `docs/user-guide.md` lists this under
the native window's limitations: "animated-image playback are not implemented;
animated GIFs and WebP files show their first frame."

The still frame comes from `load` in `src/native_frontend/media.rs`. It decodes
through `image::DynamicImage::from_decoder`, which yields one frame. The result
is wrapped as a single-frame `RenderImage` built from one `image::Frame`. GPUI's
`RenderImage` already holds a list of frames, and the `image` dependency enables
the `gif` and `webp` features, so the decoders are available but only the first
frame is used.

Expected: an animated GIF or WebP plays in its media pane, using each frame's
delay and the file's loop count. A file with one frame stays a still image and
behaves exactly as it does now.

Constraints:

- Playback must not cost anything while no animated image is visible. The
  native window draws frames only on demand on Linux
  (`context/issues/resolved/native_window_idle_redraws.md`), and idle cost is
  tracked in `context/reference/startup-performance.md`. Advancing frames must
  wake the window only while an animated image is visible in some pane, and
  stop when it is hidden, closed, left, or the window is minimized.
- Decoding stays off both UI loops, on the existing media worker, and can be
  cancelled like other media work. The current limits still apply: 128 MiB
  source files, 8192 pixels per axis, 64 MiB per decode allocation, and
  reduction to at most 2048 pixels per axis. The total memory of all decoded
  frames needs its own limit. A file over that limit must fall back to a still
  first frame or give a clear error in the pane, never an unbounded allocation.
- The base-raster cache currently holds eight results by count. Results with
  many frames must not let it grow without bound.
- Zoom, pan, fit, centering and rectangular selection belong to each pane and
  must keep working during playback. Split views of the same file keep their
  own view state.
- Frame delays of zero or close to zero, which browsers treat as a default
  delay, must not make the window redraw continuously.
- A terminal client attached to the same persistent session still shows
  `MEDIA UNSUPPORTED IN THE TERMINAL MODE`. Decoding remains in the window
  process.

Undecided:

- Which frame `y` / `Ctrl-c` copies from a selected region during playback,
  and whether a selection pauses playback.
- Whether playback can be paused, stepped frame by frame, or restarted, and
  which keys would do it. New bindings must be recorded in
  `context/reference/helix-keymap-v1.md`.
- Whether playback starts automatically or the image opens paused.

Reproduction:

1. Build with `cargo build --features native` and run
   `target/debug/runyte --window` in a workspace containing an animated GIF
   or animated WebP file.
2. Open the file from the explorer or with `:open <path>`.
3. The pane shows the first frame and does not animate.
