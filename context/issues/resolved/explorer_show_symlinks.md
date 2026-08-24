---
title: "The explorer did not mark symlinks, and Enter opened the link instead of its target"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: beec343
---

## Resolution

Commit `beec343` (`Show symlinks in the explorer and open what they point at`)
covers both halves of the report.

`DirectoryBuffer::entry_path` resolved a row to `root.join(name)` and only
checked that `fs::metadata` succeeded, so a symlink row handed the editor the
link's own path. Git then compared the buffer against the staged text of that
path, which for a link is its target string rather than the file's contents,
and every line of the opened file read as changed. `entry_path` now inspects
`fs::symlink_metadata` first and canonicalizes a link before returning it, so
Enter lands on the file Git and the language server already know about. The
root is canonicalized when the listing is opened, so only the final component
can be a link and the resolution is exact. A broken link no longer reports
`does not exist; save directory edits before opening it` — a message about
unsaved edits, which was misleading — but `<name> is a broken symlink`. A row
whose label has pending edits still fails the earlier existence check, so
navigation continues to refuse an unwritten rename.

Renaming, copying, cutting, and deleting were already correct and were left
alone: those go through `FsPlan`, which names the link, and `copy_entry`
already recreates a link rather than following it. A test now holds that
boundary in place.

The target is shown beside the name as a *hint* rather than as text. Making it
part of the projection would have made it part of what `parse_line` reads back
and what a rename edits, which is exactly what the report asked to avoid, so
hints are virtual runs like the fold marker and the inline diagnostic: they
exist only in the snapshot and can never be selected, edited, or written.

`src/row_hints.rs` is the reusable component the report asked for. A producer
supplies one string per row together with that row's display width; the module
owns the layout rule — every annotated row in a buffer starts its hint in the
same column, two cells past the longest of them, with a single space kept when
a row overruns that column — and the clipping that keeps a hint inside the
viewport without splitting a wide glyph. `Buffer::row_hints` is the one entry
point, so a later buffer kind annotates its rows the same way and lands in the
same column and colour; today only a directory buffer returns anything.
`TextRunKind::Hint` carries them across the snapshot boundary, and the
Ratatui frontend paints every hint muted and italic like the editor's other
non-document text.

The hint is read from the identity a row carries rather than from its text, so
a link keeps saying what it points at while its name is being edited, and a
row retyped into a new entry has no identity and therefore no hint. The target
is printed exactly as the link stores it, relative or absolute, without a
directory marker appended.

Deviation from the report: the separator is `→` rather than `->`, matching the
arrow the filesystem plan preview already uses for renames and moves, and
costing one cell instead of two. The report offered `->` as a suggestion
(`maybe "file.txt -> true_file.txt"?`).

Tests: `a_symlink_is_annotated_with_its_target_without_that_hint_entering_the_text`,
`opening_a_symlink_resolves_it_to_the_file_it_points_at`,
`a_symlinked_directory_opens_the_directory_it_points_at`,
`renaming_and_deleting_symlinks_works_on_the_links_themselves`, and
`a_broken_symlink_is_listed_and_reports_why_it_cannot_be_opened` in
`tests/directory_buffer.rs`; `hints_align_past_the_longest_annotated_row`,
`an_unannotated_buffer_has_no_hints`,
`a_row_past_the_shared_column_keeps_one_space`,
`a_hint_is_clipped_to_the_remaining_viewport`, and
`wide_characters_count_as_two_cells` in `src/row_hints.rs`;
`explorer_rows_carry_their_symlink_target_as_a_read_only_hint` in
`src/snapshot.rs`; and
`explorer_symlinks_render_a_muted_hint_beside_their_names` in `src/ui.rs`.

Known limitation: a symlink to a directory is listed without the `/` marker
and is not coloured as a directory, since its entry kind is `Symlink`; the
hint is what tells the two apart. Creating a symlink from the explorer remains
unsupported, as the report allowed. A hint is drawn after the row's text, so a
horizontally scrolled listing moves its hints with the text rather than
pinning them to a screen column.

## Report

The explorer should show symlinks, for example as `file.txt ->
true_file.txt`. Pressing Enter on a symlink should open the target file.
Opening a symlink instead showed the Git gutter marking every line as changed,
which is incorrect.

Renaming and deleting symlinks should work like renaming and deleting ordinary
files. The explorer does not need to support creating symlinks.

Where a symlink is displayed as `file.txt -> true_file.txt`, the `->
true_file.txt` part should be greyed out and not editable in the buffer,
working as a hint rather than as text. Such a non-editable hint would
preferably be implemented as a re-usable component if one does not exist yet,
since other buffers may want non-editable hints as well; they should then be
presented in a consistent way — colour and alignment — across buffers.
