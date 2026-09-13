---
title: "Application view action menus refused every entry after the view redrew"
status: resolved
reported: 2026-09-13
resolved: 2026-09-13
commit: bf91cb2
---

## Resolution

Commit `bf91cb2` (`Keep application action menus valid across redraws that
keep their rows`) changed what an application action menu entry remembers
about the view it was opened on.

`App::open_plugin_actions` in `src/app/plugin_views.rs` recorded the view
buffer's text revision in each `ListAction::PluginCommand`, and the list
handler in `src/app/language_workflows.rs` refused the entry with
`Application view changed; reopen the actions` whenever that revision had
moved. Every publication that changes the projected text advances the
revision, including one that only redraws a value inside a row. A view that
republishes on a timer, such as a task list showing a running elapsed-time
counter once per second, therefore invalidated its own menu within a second of
opening it, and the choice was dropped without reaching the plugin.

An entry now records the row IDs the active selection covers, or `None` when
the buffer is not a live application view, together with the query revision
it already captured. Enter is refused only when the active buffer, the covered
row IDs, or the query revision differ from what the menu saw. Row IDs are the
identity the plugin contract uses, and publication remaps selections by row
ID, so a redraw that keeps the chosen rows leaves the entry valid, while a
publication that removes or replaces a selected row, or moves the selection
onto different rows, still refuses it. The buffer text revision is not
consulted, so a row whose text changed under the same ID is still acted on.
Whether the view still offers the action, and whether the pane has presented
the current model revision, was already enforced by
`plugin_command_preflight` in `src/app/plugin_workflows.rs` when the command is
invoked, so the menu does not repeat those checks.

The row-coverage computation moved out of `submit_plugin` into
`App::plugin_view_selected_rows`, so the menu snapshot and the invocation
derive rows identically: none while a query is pending, otherwise the IDs of
every projected row a selection range touches, in ID order.
`docs/plugins/applications.md` now states that menus capture row IDs and that
a publication keeping them leaves an open menu usable.

Regression coverage is in `src/app/tests/plugin_rich_views.rs`:

- `plugin_action_menu_survives_a_redraw_that_keeps_the_selected_rows` opens
  the menu, commits two publications that change only row text, asserts the
  buffer revision moved, and confirms the action is invoked with the selected
  row and the current model revision. It fails against the previous
  buffer-revision check with the refusal message;
- `plugin_action_menu_is_refused_when_a_redraw_changes_the_selected_rows`
  removes the selected row while the menu is open and confirms nothing is
  sent;
- `plugin_action_arguments_stale_menu_is_refused_before_opening_palette` now
  commits its replacement through a helper that updates both the buffer and
  the live view, as host publication does.

`menu_entry_captured_before_query_change_is_refused_even_after_query_completes`
in `src/app/tests/plugin_view_queries.rs` continues to cover query-revision
refusal.

Known limitation: no test switches the active buffer while the menu is open,
so the buffer-identity comparison is covered only by reading. The menu is
modal, and a switch would need to come from a host-side event.

## Report

In an application view that republishes its model periodically, choosing an
entry from the `Tab` action menu did nothing. The observed case was the
ru-time task tracker plugin: with a task `in progress` and its timer running,
the task list redraws the elapsed time once per second. Pressing `Tab`,
selecting `done`, and pressing Enter left the task unchanged and the timer
running. The interaction line briefly showed
`Application view changed; reopen the actions`.

Expected behavior: the chosen action is invoked on the selected task, so
`done` marks it done and pauses its timer.

The same commands succeeded through the command palette (`::time-done`) and
through the primary Enter binding, which capture the invocation at the moment
of the key press. Every `Tab` action was affected while the view was
redrawing, not only `done`, unless it was chosen within the second in which
the menu opened. Pausing the timer first stopped the redraws and made the menu
usable.

Reproduction:

1. Open an application view whose model is republished at least once per
   second with unchanged row IDs, for example ru-time's `::time` with a task
   timer started.
2. Select a row and press `Tab`.
3. Wait more than one publication interval, choose any action, and press
   Enter.
4. Observe that the action is not invoked and the menu reports that the view
   changed.
