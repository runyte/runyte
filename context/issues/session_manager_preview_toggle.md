# The session manager preview toggle does not survive row changes

## Observed behavior

With several persistent sessions listed in the session manager (`Space Space`),
pressing `Ctrl-t` changes whether the preview column is visible. Moving the
selection through the session rows can then reverse that choice without another
`Ctrl-t` press: a hidden preview reopens, or the visible state otherwise appears
to vary with the selected host.

The behavior is easiest to reproduce when moving among several running
sessions whose previews have not all been loaded:

1. Open the session manager with `Space Space`.
2. Press `Ctrl-t` to hide the preview.
3. Move through the rows with Up, Down, `Ctrl-p`, or `Ctrl-n`.
4. Observe that the preview can become visible again without another toggle.

This makes the toggle appear to belong to individual hosts, or to change
randomly as their preview requests complete, instead of expressing one choice
for the open manager.

## Expected behavior

`Ctrl-t` controls one preview-visible state for the session manager. That state
applies to every row, regardless of which persistent host supplies the selected
session's details, and remains unchanged while:

- the selection moves between running, stopped, cached, and not-yet-loaded
  sessions;
- a selected session's asynchronous preview request starts or completes;
- elapsed activity values or refreshed session rows rebuild the visible list.

Only another `Ctrl-t` press, closing the manager, or opening a new manager may
change that choice.

## Constraints

- The selected row may still start its lazy, bounded preview request while the
  preview column is hidden, unless avoiding that work is chosen separately;
  completion must not force the column open.
- The session manager remains one picker overlay. Preview visibility must not
  become persistent-host state or a per-row preference.
- Other preview-capable pickers keep their existing `Ctrl-t` behavior.

## Regression coverage

Exercise a session manager containing multiple running and stopped rows. Hide
the preview, move onto an uncached running row, apply its preview response, and
refresh the session rows and their elapsed activity. The manager must keep the
preview hidden through every rebuild. Repeat from a visible preview to ensure
the same paths keep it visible.
