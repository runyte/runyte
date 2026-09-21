# Windows Ctrl+h and Ctrl+j act as explorer navigation keys

## Report and expected behavior

On Windows, pressing `Ctrl+h` in the explorer is treated as Backspace and
attempts to open the parent directory instead of switching panes. `Ctrl+j`
acts as Enter. Both behaviors were reported on 2026-09-21 while using the
Windows-support branch. The exact terminal version, keyboard layout, loaded
configuration and input records from the physical keypresses were not captured.

With `editor.fast_pane_keys: true`, these chords must invoke the configured
pane moves in the explorer, just as they do in other editor views. In the
default registry, `Ctrl+h` moves left, `Ctrl+j` down, `Ctrl+k` up and `Ctrl+l`
right. The initial report expected `Ctrl+h` to switch to the pane on the
right; whether that reflects a custom binding or a direction-description
mistake remains unconfirmed. The intended fix is to honor the configured
binding, not change the default directions.

Fast pane keys are disabled by default. The reported configuration has not yet
been confirmed; the reproduction below explicitly enables them. Physical
Backspace and Enter must retain their explorer actions: parent-directory
navigation and opening the selected entry respectively.

## Reproduction

Use a fixture-owned configuration containing:

```yaml
editor:
  fast_pane_keys: true
```

Open a temporary directory containing a child directory and a text file. Open
multiple panes, including an explorer with a pane to its left and another
below it. Focus that explorer in Normal mode and press `Ctrl+h` and `Ctrl+j`
separately. Pane focus should move left and down without changing the explorer's
directory or opening its selected entry. Compare with `Ctrl+w h`, `Ctrl+w j`,
physical Backspace and physical Enter, and record the terminal and keyboard
configuration. The prefixed bindings are comparison cases, not yet verified
workarounds for the reported physical setup.

## Source trace and verification boundary

The inspected code is the Phase-1 input implementation at `e600e1a`; local
unfinished Git work does not change these input files.

`src/tui/windows_input.rs::Decoder::key` preserves control-letter identity
when a native `KEY_EVENT_RECORD` supplies an H or J virtual key and a Ctrl
modifier. Records with `wVirtualKeyCode == 0` instead go through `Decoder::unit`.
That path maps both `U+0008` and `U+007F` to unmodified Backspace, and both
`U+000A` and `U+000D` to unmodified Enter, before its general control-letter
mapping. `src/tui/input.rs::convert_event` preserves those decoded identities.
The keymap therefore cannot recover the original Ctrl chord afterward.

The relevant input cases are:

| Input record | Decoded identity | Explorer interpretation with fast pane keys |
| --- | --- | --- |
| `UnicodeChar=0x08`, `VK=0`, no modifier | Backspace | Open parent directory |
| `UnicodeChar=0x0A`, `VK=0`, no modifier | Enter | Open selected entry |
| `UnicodeChar=0x08`, `VK=0x48`, `LEFT_CTRL_PRESSED` | Ctrl+h | Focus left |
| `UnicodeChar=0x0A`, `VK=0x4A`, `LEFT_CTRL_PRESSED` | Ctrl+j | Focus down |

A temporary native Windows diagnostic executed these four records through the
actual decoder and event converter with `fast_pane_keys` enabled. The native
H/J records resolved to `FocusWindowLeft` and `FocusWindowDown` in directory
scope. Feeding the two `VK=0` results into separate fixture-owned `App`
explorers opened the parent directory for `0x08` and the selected `note.txt`
for `0x0A`. The diagnostic passed (one test, zero failures). Its temporary test
hook was removed afterward; no production fix or permanent regression test is
included in this report. This verifies the table through decoder and editor
behavior rather than only source inspection.

`src/keymap.rs::FAST_PANE_MOVES` defines the pane directions, and directory
scope binds unmodified Backspace to `OpenParentDirectory`. Relevant documentation
is `docs/user-guide.md` under pane movement and the explorer, plus
`context/reference/helix-keymap-v1.md`.

This identifies a concrete decoding path matching the report. It does not yet
prove that the physical keypresses arrive with `VK=0`: capture native input in
the affected terminal to establish whether the distinction was lost upstream
or in Runyte. Legacy control bytes can be ambiguous, so source inspection alone
does not determine the correct disambiguation strategy.

## Fix constraints and acceptance

Capture the physical chords and their Backspace/Enter counterparts through
the native Windows input path, including virtual key, UTF-16 unit and modifier
state. Determine whether the outer terminal or ConPTY retains distinguishable
metadata, and select decoding or protocol changes from that evidence.

Do not globally relabel every Backspace or Enter as a pane chord. Preserve
ordinary typing, repeat handling, native Ctrl-letter records, literal bracketed
paste, pasted CR/LF content, and behavior with fast pane keys disabled. Dispatch,
help and key hints must continue to use the shared configurable keymap.

Regression coverage should include decoder records and an explorer with
multiple panes, both settings of `fast_pane_keys`, physical Backspace/Enter,
the four directional Ctrl keys, and a configured pane alias. Native ConPTY or
Windows Terminal acceptance must verify the actual transport; synthetic records
alone cannot establish physical-key disambiguation. Keep fixtures isolated and
follow the review, validation and issue-resolution conventions in `AGENTS.md`.
