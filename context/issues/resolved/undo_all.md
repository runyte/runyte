---
title: "Undoing back to saved file content leaves the buffer dirty"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: f70f966
---

## Resolution

Commit `f70f966` ("Clear dirty state when undo restores saved content") makes
`Buffer` retain a Rope-backed clean baseline and derives `dirty` from an exact
comparison of the current text to that baseline after edits, undo, and redo.
Previously `Buffer::undo` unconditionally set `dirty` to true, so the status
indicator stayed `[+]` even after it restored the original content. Saves,
save-as, reloads, discard paths, virtual replacements, and directory resets
refresh the baseline, so undo also correctly clears the indicator after a save
followed by a later edit.

`undoing_all_edits_to_the_original_content_clears_dirty` and
`undo_to_saved_content_clears_dirty_and_redo_restores_it` in `src/buffer.rs`
cover both the reported case and the save/edit/undo/redo boundary.

A later correction fixes the history boundary exposed by this change. The
buffer had recorded every Insert-mode transaction independently, while
`App::undo` replaced the text without mapping pane selections through the
inverse transaction. Insert entry now opens a history group, returning to
Normal mode commits it, and undo/redo return the applied transactions so every
pane selection and jumplist follows the changed text. `an_explicit_group_undoes_and_redoes_as_one_step`
in `src/buffer.rs` and
`an_insert_session_is_one_undo_step_and_restores_its_starting_cursor` in
`tests/selection.rs` cover the corrected behavior.

## Report

A modified file is marked `[+]` after its name in the status line. Undoing
every change left the `[+]` in place. It should disappear once the buffer
content again matches the file on disk.
