---
title: "macOS explorer clipboard paste loses line breaks"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: 7d0ad99
---

## Resolution

Commit `7d0ad99` (`Preserve line breaks in terminal clipboard paste`) corrects
`src/tui/input.rs::convert_event`. Explorer clipboard yank already retains
LF separators between selected entries. A terminal paste action can transport
those separators as lone carriage returns; the frontend passed them unchanged
into the document instead of restoring the editor's LF line separators.
The regression reproduced `alpha.txt\rβeta.txt` in place of the copied
`alpha.txt\nβeta.txt` before the change.

The frontend now converts each lone CR in a bracketed paste to LF while keeping
existing LF and CRLF unchanged. Normalization stays at the terminal event
boundary, so editor transactions, clipboard contents, and literal command
text retain their existing semantics. Single-line prompt validation still
receives control characters and rejects them. No binding changes are needed:
`Cmd-v` is the outer terminal's paste action, and `Space c y` still uses the
ordinary clipboard boundary.

`explorer_clipboard_round_trip_through_terminal_paste_preserves_rows` in
`tests/terminal_paste.rs` creates an isolated directory, selects two entries,
copies through the actual `Space c y` keys into an in-memory clipboard, models
CR transport, and verifies the pasted document and retained Insert mode.
`terminal_paste_preserves_lf_crlf_and_literal_commands` in the same file covers
mixed separators, blank lines, Unicode, tabs, and command-looking literal text.
Both failed before the fix and pass afterward on native macOS.

Known limitation: the original report does not identify the terminal
application. The regression exercises the terminal event and editor boundaries
without automating a graphical terminal or modifying the person's clipboard.

## Report

On macOS, selecting several files in the explorer, copying them with
`Space c y`, and pasting with `Cmd-v` produces a single line: the separators
between the copied entries disappear.

Expected behavior is that each copied entry retains its line break when
pasted into an editable document. The terminal's paste action must preserve
multiline text, including when it transports newline separators as carriage
returns. Existing LF and CRLF text must remain valid, and paste must remain
literal text rather than executing editor keys. Single-line prompts retain
their control-character rejection.

Reproduce by opening a directory containing multiple files, selecting several
entry lines, pressing `Space c y`, then using `Cmd-v` to paste into a document
in Insert mode. The report does not identify the outer terminal application
or specify whether the original destination was a document or a terminal.
