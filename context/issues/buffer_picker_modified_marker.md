# The buffer picker omits the `[+]` modified marker

The buffer picker (`Space b b`) does not mark buffers with unsaved changes.
`buffer_picker_columns` in `src/app.rs` appends `[STALE]` and `[RO]` to a row
label but never `[+]`, so a modified buffer is listed exactly like a clean one.
The Navigator (`Space n`) and pane titles both show `[+]` for the same buffer.

## Expected behavior

A buffer with unsaved changes carries `[+]` in the buffer picker, as it does in
the Navigator and in its pane title. `[+]` and `[STALE]` can both apply to one
buffer (edited in the editor and changed on disk); both are shown.

## Reproduction

1. Open `README.md` and make an edit without saving. The pane title shows
   `README.md [+]`.
2. Press `Space n`. The `README.md` row shows `[+]`.
3. Press `Esc`, then `Space b b`. The `README.md` row shows no `[+]`.
