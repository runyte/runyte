---
title: "Native window system clipboard copy and paste do not integrate"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 2766482
---

## Resolution

Commit `2766482` — Fix native window font sizing, clipboard integration, and build warning.

`HostPorts::live` always selected the Unix helper-based `CommandClipboard`,
while native media copying used GPUI or arboard. Ordinary editor commands
therefore still depended on external clipboard utilities, and the native
window had no Ctrl-Shift-v paste action or Ctrl-Shift-c copy binding.

Native-feature builds now use a lazily opened `NativeClipboard` port backed by
arboard. The port retains its connection so X11 clipboard ownership survives
the write call. Text and PNG reads/writes keep the clipboard byte limits, and
text takes precedence when a source also offers an image. Terminal-only builds
retain their existing platform clipboard implementation.

Ctrl-Shift-c (`Ctrl-C` in the registry) invokes the existing clipboard-yank
command in text and terminal review, or native selection copy in media. The
Terminal Insert exception and its help read the effective registry; ordinary
Ctrl-c remains child input. Copy without a review selection in Terminal Insert
preserves both the live output and the previous clipboard. Ctrl-Shift-v is a
reserved window action that reads system clipboard text and sends one bounded
literal paste, including to prompts and terminal children.

A macOS follow-up adds Cmd-c (`Super-c` in the shared key spelling) to the
clipboard-yank and media-copy registry entries. Terminal copy admission uses
that effective registry, so the new key copies review selections and preserves
live output and clipboard contents when no selection exists. Cmd-v joins
Ctrl-Shift-v as a reserved native-window literal text paste shortcut on macOS. Other Command keys retain their existing behavior.

`macos_command_c_copies_the_selection_in_all_editing_modes` in
`src/app/tests/editing_and_buffers.rs` covers the Command copy binding.
`media_select_mode_extends_native_selection_without_changing_pdf_pages` in
`src/app/tests/native_media.rs` and the terminal tests listed below also
exercise Cmd-c on macOS. `command_paste_is_macos_only_and_preserves_other_keys`
in `src/keymap/native_window.rs` checks the paste aliases and modifier
separation; `native_strokes_preserve_modal_sequences_and_modifiers` in
`src/native_frontend/tests/input.rs` covers GPUI Command-key translation.

Coverage: `terminal_copy_admission_follows_the_effective_registry` in
`src/keymap/configured.rs`;
`clipboard_copy_in_live_terminal_input_preserves_output_and_clipboard` and
`normal_mode_has_a_movable_caret_and_selects_and_copies_terminal_text` in
`tests/terminal.rs`; and
`window_font_controls_share_their_help_and_preserve_other_input` in
`src/keymap/native_window.rs`. The release binary passed
`tests/native_window.py --window-controls` with and without `--mux`, including
external reads of copied text/PNG, both copy bindings, Ctrl-v text/image paste,
Ctrl-Shift-v text paste and terminal paste. The complete X11 acceptance also
passed media text/region copying and mouse selection. The full Rust suite and
Clippy passed; canonical workspace line coverage was 92.04% (89% floor).

Known limitation: Ctrl-Shift-v follows terminal-emulator literal text paste
semantics; Normal-mode modal paste and image insertion remain on the existing
clipboard commands, including Ctrl-v. This reserved frontend paste shortcut is
not remappable. macOS and Wayland clipboard interactions were not exercised
locally.

## Report

System clipboard copy and paste are unavailable in the native window. Even
copying text with `Space c y` does not allow it to be pasted back.
`Ctrl+Shift+c` and `Ctrl+Shift+v` should provide system clipboard copy and paste
as in Alacritty, including in integrated terminals. Ordinary Ctrl+c must
remain available to terminal programs. Existing editor clipboard commands
must continue to work.

The macOS follow-up requests Cmd-c for copy and Cmd-v for system clipboard
paste, alongside Ctrl-Shift-c and Ctrl-Shift-v.
