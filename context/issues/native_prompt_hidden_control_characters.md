# Native prompts can execute pasted text that is not fully visible

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
