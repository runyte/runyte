---
title: "Mouse pane focus carried Insert mode into another pane"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: c66cb97
---

## Resolution

Commit `c66cb97` (`Leave insert when mouse focuses pane`) fixed the pointer
focus boundary. `App::handle_pointer_repeated` activated the pane under a left
press and then deliberately restored the previous Insert mode for caret
placement, without distinguishing a press inside the active pane from one
that changed panes. `App::forward_terminal_pointer` had a separate activation
path for a child using SGR mouse reporting and carried Insert mode across the
same boundary.

`App::activate_pane_from_pointer` now leaves Insert mode before a pointer press
activates a different pane, finalizing the source pane's Insert state before
focus moves. Presses inside the current pane still reposition its Insert
caret. The existing selection drag retains its source pane, buffer, and
anchor, so moving after a cross-pane press continues to form the requested
range and enters Select mode. Both ordinary editor presses and forwarded
terminal presses use the same focus rule.

Coverage lives in `src/app.rs`: `pointer_focus_leaves_insert_and_drag_selection_still_enters_select`
checks the cross-pane Normal transition and its continuing drag selection;
`pointer_respects_prompt_and_insert_ownership_and_cancels_modal_state` retains
the same-pane Insert behavior; and
`a_reported_terminal_click_focuses_its_pane_before_forwarding` covers the SGR
terminal path.

## Report

With two panes open, clicking the other pane while in Insert mode activated
that pane but kept Insert mode. A mouse click that changes the active pane was
expected to change the editor to Normal mode automatically. Click-and-drag
mouse selection was expected to remain available.
