---
title: "Animated GIF and WebP images show only their first frame in the native window"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 4269ab8
---

## Resolution

Commit `4269ab8` (`Play bounded animated images in native media panes`) replaces
`load`'s single-raster path for animated GIF/WebP with worker-side composited
frame decoding. Single-frame GIFs retain the original still decoder: the
animation decoder's additional compositing allocation would otherwise reject
large still files accepted previously. GIF loop extensions count repetitions
after the first play; absent extensions mean one play. WebP counts total plays.
A bounded metadata scan preserves that distinction and limits WebP EXIF to
64 KiB before reading its orientation. Source reads and frame iteration check
cancellation; source size, canvas dimensions, and allocation checks remain.

Each animation retains at most 1024 frames and 32 MiB after reduction. Exceeding
those limits produces a pane error. The eight-entry base cache also has a
256 MiB cap, allowing eight maximum-size results without introducing repeated
visible-source eviction. Unused GPU images are removed explicitly; evicting a
CPU raster alone does not remove GPUI atlas textures.

Playback clocks live in each viewport and render single-frame GPUI images.
GPUI's built-in animation path would request display-rate redraws irrespective
of frame delays and would not implement the file's loop count. Runyte instead
arms one timer for the earliest visible deadline through a next-frame callback.
The compositor/visibility callback gate prevents hidden windows from rearming
that timer. Empty or fully covered panes do not advance. A long unpresented
interval advances at most once rather than replaying missed frames. Delays
below 20 ms use 100 ms. Finite animations stop on their last frame.

The previously undecided interaction choices are resolved as follows: playback
starts automatically; a region selection freezes that pane's displayed frame;
copy uses that frame; clearing the selection resumes. No pause, step, or restart
keys are added. Zoom, pan, selection, and playback remain independent in split
panes. Decoding remains local to the window and changes no persistent-session
protocol or terminal-client media behavior.

Regression coverage:

- `gif_frames_normalize_delays_and_distinguish_absent_finite_and_infinite_loops`,
  `webp_frames_use_file_delays_and_total_loop_count`,
  `gif_partial_frames_are_composited_with_background_disposal`,
  `animation_frames_are_reduced_before_the_retained_budget_is_charged`,
  `still_gifs_keep_the_original_decoder_and_cancelled_sources_stop`,
  `frame_memory_count_and_canvas_limits_fail_before_unbounded_collection`,
  `playback_finishes_last_frame_and_paused_split_has_its_own_clock`,
  `playback_counts_full_repetitions_then_remains_finished_after_selection`,
  `playback_suspends_hidden_clocks_without_catching_up`,
  `fully_covered_or_empty_panes_do_not_animate`, and
  `webp_metadata_is_bounded_before_allocating_and_source_reads_are_cancellable`
  in `src/native_frontend/tests/animation.rs` cover decoding and playback.
- `animated_region_copy_uses_the_frozen_pane_frame_and_source_replacement_resets_it`
  and `large_single_frame_gif_keeps_the_still_decoder_allocation_budget` in
  `src/native_frontend/tests/media.rs` cover copying, source replacement, and
  still-image compatibility.
- `tests/native_window.py --animations`, included in native CI, checks actual
  playback, frozen-frame clipboard pixels, resume, finite-loop completion, and
  idle after selection, hiding, leaving, and completion. Each idle sample
  measured zero native-main-thread context switches per second on isolated X11;
  `context/reference/startup-performance.md` records the measurement.

The default Rust suite, native adapter tests, formatting, both lint
configurations, and existing image/PDF/editor window acceptance passed. Repeated
Astra high reviews ended with no actionable findings.

Known limitation: macOS and Wayland visibility gating was reviewed in source
but not exercised on those desktops. The retained-frame budget deliberately
rejects long or large animations; a 2048×2048 RGBA image fits only two frames.

## Report

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
