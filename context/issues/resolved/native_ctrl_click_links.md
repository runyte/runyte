---
title: "Ctrl+left-click does not open links across native views"
status: resolved
reported: 2026-10-09
resolved: 2026-10-09
commit: e50bc6d
---

## Resolution

Commit `e50bc6d` (`Follow native link clicks in documents and terminal panes`)
adds pointer navigation through the existing `gf` target resolution.

`App::handle_pointer_repeated` previously forwarded mouse events to an
SGR-reporting terminal before considering editor navigation, while document
clicks only positioned or extended selections. Native link presses now resolve
the clicked character before that forwarding step. Their drag and release stay
editor-owned even if navigation changes the pane or opens an overlay. Document
hit testing uses the existing fold-, wrap-, table-, and Unicode-aware text
projection, rejects stale pane identities, and ignores unrelated selections.
The navigation path also releases directory-tree focus, publishes errors,
updates destination history, and applies normal special-buffer retention.
Opening a file from a live terminal enters Normal mode in the document.

`TerminalSession::capture_review` now builds a read-only snapshot without
installing review state. Live clicks use that temporary snapshot; review clicks
use the existing frozen one. Both call `TerminalReview::navigation_target_at`,
extracted from the existing `gf` inference, so wrapped URLs and relative
terminal paths retain the same resolution rules. Browser navigation leaves
terminal mode, selection, scroll, revision, and child input unchanged.
Keyboard `gf` retains its explicit-selection behavior.

The platform shortcut is exact Ctrl-left-click on Linux and Windows and
Cmd-left-click on macOS, following the native platform shortcut convention
rather than using Control on every platform as the original report requested.
Extra modifiers and overlays retain their existing input behavior.

The terminal-relative-path test canonicalizes the reported directory and opened
file before comparing them with the fixture paths. OSC 7 file URIs do not retain
the Windows verbatim path prefix, so lexical equality rejected equivalent paths
in Windows CI. The assertions still require the intended directory and file.

Regression coverage in `src/app/tests/pointer_links.rs`:

- `native_link_click_uses_clicked_pane_and_markdown_label_not_selection`
- `native_link_click_rendered_markdown_follows_relative_path_and_heading`
- `native_link_click_uses_wrapped_and_scrolled_text_and_rejects_blank_cells`
- `native_link_click_obeys_overlays_and_rejects_stale_panes`
- `native_link_click_terminal_links_preserves_live_or_frozen_state_and_owns_release`
- `native_link_click_resolves_terminal_relative_paths_without_creating_review`
- `native_link_click_leaves_tree_focus_for_active_and_inactive_terminals`
- `native_link_click_reports_open_failures_and_runs_buffer_lifecycle`
- `extra_link_click_modifiers_remain_terminal_input`
- `tui_ctrl_click_still_belongs_to_terminal_mouse_reporting`

`src/terminal/tests/navigation.rs` adds
`pointer_navigation_reads_live_or_frozen_cells_without_mutating_review` and
`pointer_navigation_uses_scrollback_and_alternate_screen_without_entering_review`;
its existing wrapped-link tests also cover the shared inference used by `gf`.

A later fix gives a link click the action echo `gf` already had. Pointer
input never reached `report_completed_action`, so the interaction line kept
showing a previous key's echo. The click now takes its own action identity
and echoes `Ctrl-left-click (…)` (`Cmd-left-click` on macOS) with the
outcome, such as `opened <url> in the default browser`. A click that reaches
no text leaves the echo alone. A browser launch still pending on Windows
records that identity, so its completion or failure updates this echo, and
the echo of a keyboard `gf`, instead of leaving `Opening external
application…` behind. Covered by
`native_link_click_echoes_gesture_and_outcome_on_the_interaction_line` and
`native_link_click_echo_follows_a_pending_browser_launch_to_completion` in
`src/app/tests/pointer_links.rs`.

Known limitation: Pixel-only PDF and image previews do not expose `gf` text
targets; their existing pixel selection and pan gestures are unchanged.

## Report

# Ctrl+left-click does not open links across native views

Native Runyte supports opening file paths and web URLs with `gf`, but terminal
content requires switching from input mode to terminal review mode first
(described as preview mode in the report). Links should also be interactive
through the mouse without requiring that mode change.

Ctrl+left mouse click should perform the equivalent of `gf` on the target
under the mouse pointer in every native view that displays a supported file
path or URL. This includes terminal panes in both review and input mode.
File paths should use the existing `gf` navigation behavior, and web URLs
should open in the default browser. Target detection and resolution should
follow the existing `gf` rules, including supported wrapped terminal links.

The clicked location, rather than an unrelated caret or selection, should
determine the target. In terminal input mode, opening a link must not require
entering review mode, and the handled gesture must not also be forwarded to
the terminal child as mouse input. Existing `gf` keyboard navigation should
remain available.

To reproduce, display a file path or web URL in a native editor view and in
the output of an integrated terminal session. Verify that `gf` can open the
target, using terminal review mode where necessary. Return the terminal to
input mode and Ctrl+left-click the target. The expected behavior is the same
navigation or browser action directly from the clicked link, without first
switching modes. Repeat in terminal review and other native views displaying
supported targets.
