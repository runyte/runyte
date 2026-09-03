---
title: "The session manager preview toggle did not survive row changes"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: b709d33
---

## Resolution

Commit `b709d33` (`Preserve session preview visibility across refreshes`) fixes
`App::rebuild_workspace_picker_at`, which replaced the session manager's
`ListPicker` and restored its default visible preview whenever selected-session
data, catalog rows, or elapsed activity rebuilt the list. The rebuild now
carries the open manager's `show_preview` value alongside its filter and
selection, so asynchronous data changes cannot overwrite the choice made with
`Ctrl-t`. A newly opened manager still begins with the ordinary visible-preview
default.

`session_picker_keeps_preview_visibility_through_every_row_rebuild` in
`src/app/tests/workspace.rs` covers both visible and hidden preview states while
moving among running and stopped rows, beginning and completing an uncached
running row's preview, refreshing catalog rows, and advancing elapsed activity.

## Report

With several persistent sessions listed in the session manager (`Space Space`),
pressing `Ctrl-t` changed whether the preview column was visible. Moving the
selection through the session rows could then reverse that choice without
another `Ctrl-t` press: a hidden preview reopened, or the visible state otherwise
appeared to vary with the selected host.

The behavior was easiest to reproduce when moving among several running
sessions whose previews had not all been loaded:

1. Open the session manager with `Space Space`.
2. Press `Ctrl-t` to hide the preview.
3. Move through the rows with Up, Down, `Ctrl-p`, or `Ctrl-n`.
4. Observe that the preview can become visible again without another toggle.

This made the toggle appear to belong to individual hosts, or to change
randomly as their preview requests completed, instead of expressing one choice
for the open manager.

`Ctrl-t` is expected to control one preview-visible state for the session
manager. That state applies to every row, regardless of which persistent host
supplies the selected session's details, and remains unchanged while:

- the selection moves between running, stopped, cached, and not-yet-loaded
  sessions;
- a selected session's asynchronous preview request starts or completes;
- elapsed activity values or refreshed session rows rebuild the visible list.

Only another `Ctrl-t` press, closing the manager, or opening a new manager may
change that choice.

The selected row may still start its lazy, bounded preview request while the
preview column is hidden; completion must not force the column open. The
session manager remains one picker overlay, and preview visibility is neither
persistent-host state nor a per-row preference. Other preview-capable pickers
retain their existing `Ctrl-t` behavior.
