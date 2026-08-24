---
title: "Binary files are loaded as text instead of being handed to a program"
status: resolved
reported: 2026-07-31
resolved: 2026-07-31
legacy_commit: 3534c64
---

## Resolution

Fixed by 3534c64 "Hand binary files to a program instead of opening them as
text".

The report guessed that a binary file would open as text. What actually
happened was slightly different and no better: `Buffer::open` calls
`fs::read_to_string`, so a binary file failed the open with a generic
"stream did not contain valid UTF-8", and a binary file named on the command
line failed the whole startup. Neither said anything about a file Runyte was
never going to be an editor for.

`src/external_open.rs` answers the detection question and nothing else — it
knows about bytes, a list of strings on disk, and one process spawn, not about
buffers or drawing. A file is binary when the first eight kilobytes hold a NUL
byte or do not decode as UTF-8, the same prefix Git inspects. The read takes
one byte past that prefix, which is what separates a file ending exactly on the
boundary from a longer one: without it, a file of exactly 8192 bytes ending in
an invalid sequence would be forgiven as a character the cut had split in half,
and would go on to fail the text open anyway.

The check runs in `open_file` before the buffer is created, so a file that
cannot be saved back is never one Runyte is holding open, and every user-facing
route to a file goes through there — the file picker, `:open`, splits with a
path, jump targets, the explorer. `App::new` checks the command-line argument
separately and opens a scratch buffer with the prompt already up.

The prompt is a `PromptKind::ExternalProgram` reusing the search prompt's
editing model rather than adding a third text-entry surface. It takes a name
found on `PATH` or an absolute path, optionally with arguments split on
whitespace, which means a path containing a space has to be given bare.
Choices are remembered in the platform cache directory, most recent first and
capped at sixteen, and offered above the prompt with `↑`/`↓` and Tab; the most
recent one seeds it. The location remains per-user rather than in the workspace
`.runyte`, preserving the original boundary: this is a record of a person's
tools, not of a project's state. Linux and other XDG systems use
`$XDG_CACHE_HOME/runyte` or `~/.cache/runyte`, while macOS uses
`~/Library/Caches/runyte` when XDG is unset.

The program is spawned detached with no stdio of its own, because Runyte owns
the terminal in raw mode and a child sharing it would corrupt the screen and
compete for keystrokes. It is reaped on a thread so a session spent opening
images does not leave one zombie per image. A program that fails to spawn is
reported and deliberately not remembered, and a binary file arriving while the
prompt is already up — a language server answering a goto, say — is refused
rather than allowed to replace the question and silently inherit a half-typed
answer.

Tests: `text_is_not_binary_and_nul_bytes_are`,
`a_character_split_by_the_prefix_boundary_is_still_text`,
`a_file_ending_on_the_prefix_boundary_is_read_as_complete`,
`a_binary_file_on_disk_is_detected_and_a_text_one_is_not`,
`cache_paths_follow_platform_conventions_and_honor_xdg`,
`relative_xdg_cache_home_is_ignored`,
`remembering_moves_a_program_to_the_front_and_survives_a_reload`,
`the_cache_is_bounded_and_ignores_empty_choices`, and
`a_cache_with_no_home_still_remembers_in_memory` in `src/external_open.rs`;
`opening_a_binary_file_asks_for_a_program_instead_of_a_buffer`,
`a_chosen_program_is_remembered_and_offered_back_as_a_hint`, and
`a_program_that_cannot_run_is_reported_and_not_remembered` in `src/app.rs`;
and `a_binary_argument_opens_the_open_with_prompt_over_its_hints` in
`tests/key_hints.rs`. Every test that writes to the cache points it at a
temporary directory, never at a real home.

Known limitation: a terminal program cannot take the screen over, since the
child gets no terminal — the intended targets are viewers and GUI
applications. The cache is written with a plain `fs::write`, so two Runyte
instances choosing programs at the same moment can lose one update; it is a
hint list, not durable state. `buffer_for_path`, used only when a language
server names files for a `WorkspaceEdit`, does not run the check, because those
paths come from a refactor rather than from browsing.

## Report

Opening a binary file appeared to load it as text. The requested behavior:

- Opening a binary file raises a prompt asking which program to open it with.
- The program is typed manually, either as an absolute path or as a bare name
  resolved through `PATH`.
- Recently chosen programs are cached and offered as hints.
- That cache lives in `~/.runyte`.
