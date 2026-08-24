---
title: "Splitting in an explorer opens the entry under the caret instead of the buffer"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: cb9982d
---

## Resolution

Fixed by cb9982d "Split an explorer the way any other buffer splits".

The directory scope owned six bindings of its own: `Ctrl-w v`, `Ctrl-w Ctrl-v`
and `Space w v` reached `open-directory-entry-vertical`, and the `s` spellings
reached `open-directory-entry-horizontal`. Both went through
`open_directory_entry`, which took an `Option<Axis>`: with an axis it called
`split(axis, Some(path))` so the new pane opened the entry, and with no entry
on the row it returned early with "no directory entry on this row" — the split
never happened at all. Because a directory binding shadows a global sequence
only inside that scope, those six rows were the sole reason splitting meant
one thing in an explorer and another everywhere else.

The fix deletes the six bindings and the two command identities behind them,
and `open_directory_entry` loses its parameter. Nothing replaces them: the
global `split-vertical` and `split-horizontal` bindings now apply in a
directory buffer, and `split(axis, None)` already copies the active pane, so
the new pane shows the same listing on the same row. An empty row is no longer
a reason a split cannot be made, because no entry is looked up.

The report worried this would cost the copy-across-two-explorers workflow. It
does not, and no extra buffer is created to protect it. `split` clears the new
pane's `directory_buffer`, so the first time that pane navigates,
`pane_directory_buffer` finds the shared listing claimed by the pane it was
split from and opens a fresh explorer instead of retargeting one somebody else
is browsing with. Two panes therefore share one directory buffer only while
they are showing the same directory — exactly like two panes on one file — and
become independent explorers as soon as one of them moves, which is when a
copy or cut between them starts to mean anything.

Covered by `splitting_an_explorer_shows_the_same_listing_in_both_panes` in
`src/app.rs`, which splits an explorer twice, splits again from a row with no
entry on it, and then navigates one split to confirm it takes an explorer of
its own while the other stays where it was.

Known limitation: while both panes still share the listing, an edit to the
directory buffer in one is an edit in the other, since it is one buffer. That
is the same behavior two panes on one file have always had, and navigating
either pane separates them.

## Report

Vertical and horizontal splits in the explorer opened the file or directory
under the cursor in the new split. With the cursor on an empty explorer line,
a split reported that no file was under the cursor.

Splits should behave consistently with files: a new split shows the same
buffer, leaving the choice of what to do with it to the user. The existing
ability to copy files between two explorer buffers should be preserved, which
a split creating a new buffer on the same directory would satisfy. The
behavior was what mattered; the mechanism was left open.
