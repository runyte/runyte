---
title: "Zoomed native PDF pages change pages instead of panning"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 0079463
---

## Resolution

Commit `0079463` (`Pan zoomed PDF pages and reset fit before leaving previews`)
corrects `App::handle_media_command`, which previously left PDF vertical
motions to the ordinary page-buffer cursor even when the native viewport was
zoomed in. The Media registry now exposes shared vertical-motion commands;
Normal mode sends them to the native viewport, which pans above fit size and
requests adjacent pages at fit size or below. Select mode still extends PDF
word/line selections. Counts repeat vertical motions, and file-boundary page
jumps retain their existing behavior. Retained media in a terminal attachment
continues using ordinary page-row motion without waiting for a native viewport.

The Media scope also binds Ctrl-n/Ctrl-p to the existing next/previous-page
commands. Ctrl-f/Ctrl-d/PageDown and Ctrl-b/Ctrl-u/PageUp retain their page
behavior at every zoom. Help, hints, and execution use the same registry.
Private bundled protocol version 75 adds the vertical media actions, so older
hosts and newer frontends cannot silently disagree about them.

`Viewport::back` now returns a zoomed-in PDF to fit in one press while keeping
its selection. Later presses clear the selection, return to the page buffer,
and open the source directory. Overlays and pending key sequences still dismiss
first. These choices resolve the report's open questions: j/k keep page motion
at fit size, Escape resets directly to fit, and images keep their previous
Escape behavior. Queued vertical actions cannot operate on a different PDF
page from the one captured with the request.

Regression coverage:

- `native_media_bindings_use_the_registry_and_preserve_pdf_page_motions`,
  `pdf_page_shortcuts_and_counted_vertical_requests_use_media_scope`,
  `media_select_mode_extends_native_selection_without_changing_pdf_pages`, and
  `pdf_back_opens_page_buffer_with_ordinary_motions_and_enter_previews` in
  `src/app/tests/native_media.rs` cover command dispatch, counts, Select mode,
  prefix cancellation, terminal fallback, and page-buffer navigation.
- `pdf_escape_fits_before_clearing_selection_and_leaving`,
  `vertical_pdf_motion_pans_only_above_fit_and_images_never_change_pages`, and
  `escape_clears_text_or_region_before_leaving_without_resetting_view` in
  `src/native_frontend/viewport.rs` cover zoom, selection, and image behavior.
- `tests/native_window.py` exercises zoomed panning, all six control-key page
  shortcuts, selection retention on fit, and subsequent page-buffer/explorer
  navigation. Both its standalone and `--mux --no-system-fonts` runs passed;
  CI runs the full acceptance scenario in both attachment modes.

## Report

In the `--window` native frontend, a displayed PDF page uses `j`/`k` for page
navigation at every zoom level, while `h`/`l` pan horizontally. Vertical
panning of a zoomed-in page needs `z j`/`z k`, the wheel, or a drag, so `hjkl`
do not move around a zoomed page as a single set. For a PDF,
`handle_media_command` in `src/app/file_workflows.rs` maps
`MoveLeft`/`MoveRight` to panning but leaves `MoveUp`/`MoveDown` to the
ordinary motion over the page buffer rows.

Page keys today: `Ctrl-f`/PageDown and `Ctrl-d` show the next page, and
`Ctrl-b`/PageUp and `Ctrl-u` the previous page (the PDF branch of
`handle_media_command` turns `PageDown`/`HalfPageDown` and
`PageUp`/`HalfPageUp` into one-page motions). `Ctrl-n` and `Ctrl-p` have no binding in the media scope.

Escape today: dismiss an overlay or pending key sequence, then clear a media
selection, then leave the displayed page for the PDF page buffer, then open the
source directory with the PDF selected. Zoom is not part of this sequence, so
Escape on a zoomed-in page leaves it at the current zoom.

## Expected

On a displayed PDF page in Normal mode:

| Input | Action |
| --- | --- |
| `h` / `j` / `k` / `l` | Pan left / down / up / right on a zoomed-in page |
| `+` / `-` | Zoom in / out (unchanged) |
| `Ctrl-f`, `Ctrl-d`, `Ctrl-n` | Next page |
| `Ctrl-b`, `Ctrl-u`, `Ctrl-p` | Previous page |

Escape on a zoomed-in page proceeds in this order:

1. zoom out;
2. clear the selection;
3. return to the PDF page buffer;
4. open the source directory (explorer).

## Undecided

- Whether `j`/`k` keep moving between pages when the page is at fit size, or
  pan (and do nothing) there as well. The request names panning only for a
  zoomed-in page.
- Whether the Escape zoom-out step returns to fit size in one press or steps
  out one zoom level per press.
- Whether dismissing an overlay or pending key sequence stays ahead of the
  zoom-out step. It is first today.
- Whether images adopt the same Escape zoom-out step. The request covers PDFs;
  images already pan with `hjkl`.

## Constraints

- Select mode (`v`) keeps `h/j/k/l` as PDF word and line selection extension.
- The new `Ctrl-n`/`Ctrl-p` bindings and any changed `j`/`k` or Escape
  behavior go through the `Media` binding scope in `src/keymap.rs`, so
  dispatch, help, and key hints stay in agreement.
- The media keys table in `docs/user-guide.md` and the "Experimental native
  window" section of `context/reference/helix-keymap-v1.md` describe the
  current `j`/`k` and Escape behavior and need to change with it.

## Reproduction

1. `runyte --window some.pdf`.
2. Press `+` a few times to zoom in.
3. Press `j`: the next page is shown instead of panning down.
4. Press `Ctrl-n`: nothing happens.
5. Press Escape: the page buffer is shown; the zoom is not reset first.
