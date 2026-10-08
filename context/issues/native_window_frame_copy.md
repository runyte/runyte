# Native window copies the whole cell grid for every host frame

Every frame the host publishes to the native window (`--window`) copies the
entire cell grid, even when one cell changed.

## Measurements

Release build, per keystroke in Insert mode on a plain-text buffer, median and
p99 over 500 keys. The host was driven directly through `WorkspaceHost`, with
no display.

| Grid | Cells | `Terminal<TestBackend>::draw` | `Buffer::clone` |
| --- | ---: | ---: | ---: |
| 120×40 | 4,800 | 0.19 / 0.37 ms | 0.07 / 0.11 ms |
| 213×54 | 11,502 | 0.37 / 0.71 ms | 0.15 / 0.21 ms |
| 426×108 | 46,008 | 0.93–1.10 / 1.65 ms | 0.40–0.50 / 0.72 ms |

Frame preparation (`prepare_frame_with_hints`) took 0.14–0.37 ms median over
the same range and is shared with the terminal frontend.

## Diagnosis

`native_frontend::Surface::draw` renders through a `ratatui::Terminal` over a
`TestBackend`. Ratatui renders into its own buffer, diffs it against the
previous one, and writes the changed cells into the `TestBackend` buffer.
`Surface::draw` then clones the complete `TestBackend` buffer into
`FrameData::cells` and hands it to the GUI thread. The diff already
identifies the few rows a keystroke changes, but the copy is always the full
grid, and the previous `FrameData` grid is freed when it is replaced.

## Expected behavior

The cost of publishing a frame follows what changed rather than the grid
size. A frame that changes a few rows copies only those rows; unchanged rows
are shared between consecutive frames. The GUI thread still receives a
complete, immutable frame that it may keep for as long as it needs.

## Constraints

- The bridge keeps only the latest frame and never queues frames; a skipped
  frame must not lose changes, so every published frame stays complete.
- Frames remain immutable once published: the GUI thread may still be
  painting a frame while the host prepares the next one.
- Painted-frame identity, attachment checks and acknowledgements are
  unchanged.
- Standalone windows and persistent-session attachments share
  `Surface::draw`, so both benefit.
- Sharing unchanged rows also lets the cell painter recognise unchanged rows
  between frames (`native_window_cell_paint_cost.md`).

## Reproduction

Time `Surface::draw` with `RUNYTE_NATIVE_PAINT_TIMING`-style instrumentation,
or reproduce the table with a release benchmark that drives
`WorkspaceHost::execute_frontend_input` and renders with
`runyte::ui::render_native_frame` into `Terminal<TestBackend>` at each grid
size.
