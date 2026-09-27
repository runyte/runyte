# Selection undo and redo

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
