---
title: "Screenshot paste with Ctrl-v has no visible effect in a Windows buffer"
status: resolved
reported: 2026-09-21
resolved: 2026-09-22
commit: 390c790
---

## Resolution

Commit `390c790` (`Handle screenshot paste in Windows Terminal`) addressed
Windows Terminal's ownership of `Ctrl-v`. The terminal's default paste action
receives that key before Runyte does. For image-only clipboard content, some
Windows Terminal builds send an empty bracketed-paste event, while others send
no event. Runyte's standalone event loop previously passed an empty paste to
`App::handle_text` as empty text, so the clipboard image command never ran.

The initial empty-paste route synthesized `Ctrl-v`, which could complete a
pending editor sequence such as `Ctrl-w Ctrl-v` and split the pane. A later
correction routes an empty paste in an editor buffer to a semantic clipboard
event, independent of pending keys and configured bindings. It leaves terminal
panes, input overlays, command prompts, and nonempty text paste alone. `Alt-v`
is also bound to the same
clipboard-paste command in Normal, Select, Insert, and Replace so an image can
be pasted when Windows Terminal emits no event for `Ctrl-v`. The existing
`Ctrl-v` binding remains for terminal hosts that deliver it directly. The
keymap register and user guide document the alternate spelling; terminal
panes still pass these keys to their child when the outer terminal sends them.

Coverage is in `src/main.rs`:
`empty_windows_image_paste_becomes_a_semantic_event_only_in_editor_panes`;
`src/tui/windows_input/tests.rs`:
`alternate_image_paste_key_survives_native_console_input`; and
`src/app/tests/editing_and_buffers.rs`:
`semantic_image_paste_ignores_pending_key_sequences_and_operands` and
`alt_v_pastes_an_image_when_the_outer_terminal_reserves_ctrl_v` alongside
`ctrl_v_stores_a_clipboard_image_and_writes_a_numbered_link`.

Known limitation: if the outer terminal consumes `Ctrl-v` and emits no input,
Runyte cannot observe that key; use `Alt-v`. The reporter's exact screenshot
tool, Windows Terminal version, and clipboard formats were not captured, so
the original physical gesture was not replayed on that installation.

## Report

On Windows in Windows Terminal, pressing `Ctrl-v` with a screenshot on the
clipboard in an editable Runyte buffer had no visible effect: no image link or
text appeared. The screenshot tool, buffer mode, and clipboard formats were
not recorded.

For an image-only clipboard, the expected behavior is to store the image under
the workspace's `.runyte/cache/images/` directory and insert a numbered
Markdown link such as `[Image 1](...)` into the buffer. Clipboard text takes
precedence when both text and image formats are offered. Reproduce by copying
a screenshot, focusing an editable buffer in Runyte inside Windows Terminal,
and pressing `Ctrl-v`.
