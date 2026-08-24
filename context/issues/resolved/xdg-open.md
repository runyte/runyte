---
title: "Binary-file prompt has no system-default application choice"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: 486680b
---

## Resolution

Commit `486680b` (`Open binary files with the system default`) resolves this
issue. `App::ask_for_external_program` was seeding the prompt with the most
recent custom program, while `App::open_externally` rejected an empty answer;
there was therefore no prompt state representing the desktop's preferred
application. `external_open::launch` also had no platform-default abstraction.

The chooser now contains `xdg-open` on Linux or `open` on macOS as an actual
system-opener row, selected by default. This replaced an intermediate design
that represented the system opener only as an empty command while visibly
selecting the first remembered row; in that design, the prompt and highlight
gave Enter contradictory meanings. Enter now always opens with the selected
row. Typing still filters remembered applications or supplies a new one.

Explicit programs are cached separately from the selected default. Tab on a
remembered row opens actions to delete it or make it the persistent default;
deleting a custom default restores the platform opener. Both the standalone
terminal UI and persistent-host snapshots expose the same rows, selection,
labels, and action window. The README documents the complete interaction.

Commit `01761aa` (`Track incomplete Windows support work`) separately created
the requested provisional Windows issue. It makes a whole-source compatibility
scan the first task and records the deliberately limited support and testing
expectations.

Tests covering the behavior are:

- `system_default_openers_follow_desktop_platform_conventions` in
  `src/external_open.rs`
- `an_empty_choice_uses_the_system_default_but_an_explicit_one_wins` in
  `src/external_open.rs`
- `a_custom_default_and_program_deletion_survive_reload` in
  `src/external_open.rs`
- `opening_a_binary_file_asks_for_a_program_instead_of_a_buffer` in
  `src/app.rs`
- `a_chosen_program_is_remembered_and_offered_back_as_a_hint` in `src/app.rs`
- `a_binary_argument_opens_the_open_with_prompt_over_its_hints` in
  `tests/key_hints.rs`

Known limitation: Windows has no system-default opener in this change. That is
deliberately deferred to `context/issues/windows_support.md`.

## Report

Binary files were opened through an application supplied by the user. The user
had to type the application name, although previous applications were
remembered and suggested.

On Fedora, `xdg-open` opens a file in the user's preferred application. The
equivalent behavior on macOS needed to be determined. If macOS supported the
same concept, the application prompt was to remain, but its default action was
to use the system-preferred application, with wording such as "press enter to
use system default app".

Windows support was deferred for future work. A separate
`windows_support.md` issue was requested, with the system-default binary-file
opener as its first known item. That issue was to state that its list was
incomplete and that the first task must be a scan of the Runyte source for all
Windows-compatibility issues. Windows did not need first-class support, and
features could be skipped there if their implementation became difficult.
Only limited Windows testing was available.
