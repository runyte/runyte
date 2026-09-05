---
title: "Branch and tag refs were appended to the commit title as text in the Git log"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 75a2857
---

## Resolution

Commit `75a2857` (`Show a commit's branch and tag refs as a row hint, not row
text`) stops `App::open_git_log_result` from string-appending
`" ({decorations})"` onto a commit's subject when building each Git-log row.
`CommitSummary` already kept `subject` and `decorations` as separate fields
parsed from Git's independent `%s` and `%D` format fields, so the row text
built by `open_git_log_result` is now only
`"{hash}  {date}  {author}  {subject}"`; a commit's refs no longer become
buffer text a person can select, search, or copy alongside the subject.

`Buffer` gained nowhere to keep that per-row decoration text once it left the
row's own line, unlike a directory buffer, which already carries symlink
targets by row identity. `Buffer` now has a `git_log_hints` map from buffer
line to decoration text, and a new `Buffer::replace_git_log_text` sets it
together with the row text on every refresh, so a hint can never survive past
the row it described. `Buffer::row_hints` — the entry point the explorer's
symlink hints already go through, and which the Git log already used for its
own paging reminder on the heading line — now also emits one entry per
decorated commit row, so refs render through the same
`RowHints`/`TextRunKind::Hint` path: muted, italic, aligned to one shared
column, and never part of the row's text. Because the heading and the commit
rows now share one `RowHints` instance, the paging reminder's column shifts
out to match the widest hinted row whenever the page has at least one
decorated commit; a page with no decorated commits renders exactly as before.

`Space g f`'s commit picker (`open_git_commit_search_result`) never appended
decorations to its rows and needed no change.

Tests: `git_log_shows_branch_and_tag_refs_as_a_row_hint_not_text` in
`src/app.rs`, which checks that a decorated row's text carries no ref
substrings, that `row_hints().text()` holds the expected hint, that the
rendered snapshot carries it as a `TextRunKind::Hint` run, and that
refreshing a row to no longer have a ref clears its hint rather than leaving
it stale. The existing `log_pages_step_forward_and_back_without_taking_a_motion_key`
in `src/app.rs`, which exercises the undecorated case, was left passing
unmodified. The muted/italic styling itself is already covered generically by
`explorer_symlinks_render_a_muted_hint_beside_their_names` in `src/ui.rs`.

Follow-up: `RowHints::aligned` lines every hint in a buffer up in one shared
column past its longest annotated row, which is what the paging reminder and
every commit's ref hint originally used. A commit subject has no length
limit, so a single long, decorated commit pushed that shared column out far
enough to be off-screen on anything but a very wide pane, hiding the paging
reminder and every other commit's ref hint along with it — not only the long
row's own hint. `RowHints` gained a second constructor, `RowHints::trailing`,
that sits a hint one space past its own row's text with no shared column, and
`Buffer::row_hints` now builds every Git-log hint that way instead of with
`RowHints::aligned_with_gap`, so a row's hint depends only on that row, never
on how long any other row's text or hint is. Symlink hints in the explorer
keep using `RowHints::aligned`, where a shared column is still the better
read since filenames stay close enough in length for it to look like a
table. Tests: `trailing_hints_sit_one_space_past_each_rows_own_text_regardless_of_others`
and `trailing_skips_empty_hints_and_drops_them_from_the_map` in
`src/row_hints.rs`; `one_very_long_decorated_commit_does_not_hide_hints_on_a_narrow_pane`
in `src/app.rs`, which renders an 80-column pane holding a 2000-character
decorated subject alongside ordinary short rows and confirms every hint still
renders.

## Report

`Space g l` shows Git commits, for example:

```
ddf57b3a02c6  2026-08-18  Example Author  New issue: less space g commands (HEAD -> dev)
a4c5532baf09  2026-08-18  Example Author  Merge: noisy error notifications (main)
7af27c9e7009  2026-08-18  Example Author  Resolve noisy error notifications issue (fix-a67698a0acff16516)
6b12c75b2032  2026-08-18  Example Author  Gate optional LSP requests on advertised capabilities; stop retaining No binding
9c67b7818502  2026-08-18  Example Author  Merge: error text in the interaction line
5327edc52fc8  2026-08-18  Example Author  Resolve interaction-line message issue (fix-a1c7f915a284d7967)
86ae080051c7  2026-08-18  Example Author  Carry a failed or unavailable action's message onto the interaction line
782299235b49  2026-08-18  Example Author  Merge: --cwd-file is an exposed implementation detail
26c3b7bb133c  2026-08-18  Example Author  Resolve cwd_file_is_an_exposed_implementation_detail (fix-a5cc54060a9afc8c9)
99b0e958bd70  2026-08-18  Example Author  Stop documenting --cwd-file as an option anyone should pass by hand
387286623e3d  2026-08-18  Example Author  Report unsupported language-server requests
71cd06191220  2026-08-18  Example Author  Report command, keybinding, and completion cleanups
da6934f36aa6  2026-08-18  Example Author  Merge branch 'dev' (origin/main, origin/HEAD)
318ff70c8df0  2026-08-18  Example Author  Merge branch 'editor-fixes'
c8431c0b9edc  2026-08-18  Example Author  Resolve background syntax reparse issue
00227ebe161f  2026-08-18  Example Author  Move syntax reparsing off the input path
8158b30463d9  2026-08-17  Example Author  Resolve default workspace names issue (editor-fixes)
```

Some commit titles are appended with the information which branch sits on
that commit.

The report asked for this information to be presented as a non-editable text
hint, using the same UI element as symlinks in the explorer.
