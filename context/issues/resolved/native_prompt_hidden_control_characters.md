---
title: "Native prompts can execute pasted text that is not fully visible"
status: resolved
reported: 2026-09-17
resolved: 2026-09-17
commit: cfbe6de
---

## Resolution

Commit `cfbe6de` (`Reject hidden control characters in native prompts`) fixes
`App::handle_text`, which previously inserted text verbatim into single-line
prompts and filters. A shared guard follows literal-input ownership and rejects
the complete insertion before any value, cursor, selection or confirmation
revision changes. It covers command/search/rename and other native text prompts,
Finder, filterable lists, session-directory input and typed Git confirmations.
Synthetic character-key events use the same guard. Command and search submission
also check prefilled or completed values before trimming or executing them.

Rejection reports `Control characters are not allowed; nothing was inserted`.
Submit-time rejection reports `Control characters are not allowed; input was not
submitted`. Only static feedback is retained; rejected text is never copied into
it. The next non-pointer input clears feedback, and clean paste recovers in place.
Buffer editing and terminal paste keep their existing behavior; ordinary terminal
paste still translates LF to CR, while bracketed terminal paste keeps its framing.
Literal backslash sequences such as `\n` remain accepted. Trailing line endings
are rejected with other controls rather than silently stripped from paths.

`App::overlay_snapshots` carries feedback alongside existing guidance. Search and
rename receive a panel above the interaction line. The standalone renderer uses
these complete snapshots during rejection because its older drawing helpers omit
overlay messages. Both renderers reserve wrapped message rows, including in
populated Finder previews, so the error and previous guidance remain visible.

A real-editor PTY run confirmed whole-command rejection, preservation of an
existing `open ` prefix, clean recovery to the exact `alpha` path, and visible
search and Finder feedback. The newline-containing `alpha\nbeta` target was
never opened. Temporary harness artifacts are not repository dependencies.

Validation: With both follow-up fixes present, `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings` and `cargo test --locked`
passed (3,654 passed, 34 ignored). The canonical
`cargo llvm-cov --locked --workspace --summary-only --fail-under-lines 89`
run also passed and measured 91.83% line coverage on
`x86_64-unknown-linux-gnu`, above the unchanged 89% floor. Process/socket
checks ran outside the sandbox without an outer `XDG_CONFIG_HOME` override.
Native macOS coverage remains a CI check.

Regression coverage in `src/app/tests/prompt_input.rs`:

- `command_paste_cannot_open_a_hidden_newline_suffix_and_clean_paste_recovers`
- `single_line_prompt_kinds_reject_controls_preserve_cursor_and_allow_literal_escapes`
- `prefilled_command_and_search_controls_are_rejected_on_submit`
- `finder_and_list_pastes_preserve_query_selection_and_buffer`
- `session_directory_paste_preserves_completion_state`
- `typed_git_confirmations_reject_controls_without_changing_authorization`
- `multiline_buffer_paste_remains_one_literal_edit`
- `rejected_paste_feedback_is_drawn_in_standalone_and_attached_frontends`

The screen regression covers populated lists, 120×40 and 60×20 terminals, wrapped
feedback, preserved path guidance, bottom placement and clean recovery in both
frontends. `multiline_text_paste_reaches_the_terminal_without_prompt_validation`
in `tests/terminal.rs` verifies the real terminal child receives multiline/tab
input with its established LF-to-CR translation.

## Report

Native single-line input accepts control characters in a bracketed paste. The
command line renders only up to an embedded newline, but submission parses the
complete stored string. Pasting `open alpha\nbeta` therefore displays
`:open alpha` before Enter and opens a buffer whose path contains the newline
and `beta` after Enter. The executed argument is not fully visible to the user.

Reproduction uses a real editor over a PTY in a temporary project. Enter the
command prompt with `:`, send `ESC [ 200 ~ open alpha LF beta ESC [ 201 ~`,
then press Enter. The command opens the newline-containing path rather than
rejecting the paste. The visible action begins `:open (opened .../alpha...)`,
and `beta` appears elsewhere on screen.

`App::handle_text` inserts command text verbatim. Its Git branch-switch,
branch-deletion and worktree-removal confirmation paths use
`insert_confirmation_text` without a control-character check, and filterable
lists push each pasted character directly into their filter. Finder and
session-directory queries also need to be included in the input audit.

Expected behavior is atomic rejection of control-containing text destined for
native single-line input, with visible feedback that identifies the cause and
preserves the previous value, cursor and selection. A clean subsequent paste
must recover in place. No submitted command should execute a hidden pasted
suffix. Rejected text must not be included in error messages.

Ordinary buffer editing and terminal input are multiline and must retain their
existing literal-text behavior. Literal backslash sequences such as `\n` in a
regex are ordinary characters and remain valid. Plugin text and secret fields
already reject controls atomically; their fix is recorded in
`resolved/plugin_prompt_pasted_control_characters.md`.
