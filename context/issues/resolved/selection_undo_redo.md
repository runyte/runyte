---
title: "Accidental motions lose selections without a selection undo history"
status: resolved
reported: 2026-09-27
resolved: 2026-09-27
commit: c918ddc
---

## Resolution

c918ddc (`Add bounded selection undo and redo`) adds selection history
independently of buffer text undo. `App::undo` and `App::redo` map selections
through text transactions; they did not retain selection-only operations, so
an accidental motion could not recover a constructed multi-selection.

`app::selection_history::History` retains complete ranges, direction, primary
range, selection semantics, Normal/Select mode and transient whole-line state.
Each pane owns separate histories for its buffers. New splits start empty and
closed buffers release their histories. Buffer revision checks reject history
after any text change, including shared-pane edits, text undo/redo and whole
buffer replacement. Restoring history never edits text or enters Insert/Replace.

Recording surrounds completed input and semantic command actions, rather than
individual selection assignments. Nested execution and counted motions form
one entry; pointer press through release or cancellation forms one entry,
including edge autoscroll. A bounded prompt origin groups prompt opening,
acceptance and cancellation. Cancelled prompts do not add history or clear
redo. New selection changes discard the redo branch; no-ops preserve it.

The commands are `selection-undo` and `selection-redo`, bound to `Alt-u` and
`Alt-U` with discoverable `Space s u` and `Space s U` spellings. They were
also colon commands until the palette was narrowed to commands looked up by
name; selection editing is now reached only through keys and configured
bindings, and the prompt-origin redo source that existed only for the typed
spelling was removed with it. These are Runyte additions, not default Helix
bindings. The original report did not settle the text-edit policy; this
implementation stops at text revision boundaries rather than mapping old
selections through edits or mixing selection changes into text undo.

History is bounded to 128 states and 4,096 aggregate ranges per buffer, for up
to 32 buffers per pane. Oversized selections end their history. Read-only buffers
are supported. No text snapshots, persistent files or background tasks are added.

Regression coverage in `src/app/tests/selection_history.rs`:

- `selection_history_recovers_ranges_direction_primary_and_mode`
- `selection_history_groups_counts_and_preserves_redo_on_noop`
- `selection_history_restores_whole_line_delete_behavior`
- `selection_history_aliases_and_semantic_execution_share_history`
- `selection_history_search_acceptance_is_one_step_and_cancel_is_noop`
- `selection_history_is_independent_in_splits_and_invalidated_by_shared_edits`
- `selection_history_survives_buffer_visits_but_not_text_undo`
- `selection_history_is_bounded_and_works_in_read_only_buffers`
- `selection_history_groups_mouse_drag_and_keeps_pointer_semantics`
- `selection_history_bounds_range_storage_and_breaks_on_oversized_selections`
- `selection_history_bounds_retained_buffers`
- `selection_history_replayed_keys_and_counted_history_commands`
- `selection_history_does_not_reenter_insert_or_restore_across_replacement`
- `selection_history_redo_restores_line_and_select_modes`
- `selection_history_has_no_typed_spelling_and_a_refused_one_keeps_history`
- `selection_history_cancelled_prompts_after_line_selection_preserve_redo`
- `selection_history_search_from_lines_is_one_step_and_restores_line_behavior`

`selection_history_groups_autoscroll_and_keyboard_cancelled_drag` in
`src/app/tests/mouse_autoscroll.rs` covers the complete autoscrolled gesture
without a mouse-up event. Registry inventories remain checked by
`command_inventory_classifies_every_command_and_current_binding` in
`src/app/tests/commands.rs` and
`every_complete_hint_description_fits_forty_four_terminal_cells` in
`src/key_hints.rs`.

Known limitation: live terminals and terminal review do not have selection
history. Ordinary generated terminal-output buffers support it like other
read-only buffers. History cannot recover selections from earlier text revisions.

## Report

Reported in [GitHub issue #4](https://github.com/runyte/runyte/issues/4),
“Soft undo/redo (de)selection”. The report links to
[Helix issue #1596](https://github.com/helix-editor/helix/issues/1596).

A selection constructed with several character-find commands can be lost by an
accidental cursor motion. Ordinary undo and redo address text edits and do not
provide a history of selection-only changes. Recovering the previous selection
currently requires reconstructing it.

Selection undo and redo should restore the complete multi-selection, including
range direction and the primary range, without changing buffer text. Histories
must be independent for separate panes displaying the same buffer. The upstream
discussion considers several policies for crossing text edits; the original
Runyte report does not choose a policy or propose a keybinding.

Reproduction: select several words between semicolons using successive
character-find commands, then accidentally move the cursor. The earlier
selection cannot be recovered with a dedicated selection-history command.
