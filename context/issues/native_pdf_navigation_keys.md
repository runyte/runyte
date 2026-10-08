# PDF navigation keys in the native window

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
