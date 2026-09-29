The directory tree introduced for [GitHub issue #6](https://github.com/runyte/runyte/issues/6)
lacks several navigation actions available elsewhere in the editor:

- `gg`/`ge`/`G` and related page and viewport motions do not navigate tree rows.
- `.` does not show or hide hidden files.
- `/` does not search the tree.

With the tree focused through `Space d d`, those keys should operate on the
tree rather than the ordinary pane behind it. The report does not specify
whether search should traverse collapsed directories or filter visible rows,
or which keys should repeat a search given that `n` creates an entry.

Additionally, `:q`, `:wc`, and `:close` should close the focused directory tree
and return focus to the previous pane. They must preserve the other panes,
open buffers, unsaved text, and terminal sessions. The existing tree retains
its navigation state when hidden, and Escape returns focus without hiding it.
