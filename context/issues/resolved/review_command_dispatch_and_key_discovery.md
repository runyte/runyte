---
title: "Command dispatch and discovery disagreed on counts, command inventory, and reload behavior"
status: resolved
reported: 2026-08-25
resolved: 2026-08-25
commit: a4f5441
---

## Resolution

Commit a4f5441 (`Harden command dispatch and discovery`) resolved three
behavioral inconsistencies and strengthened the registry audits that guard
them.

`RunyteGrammar::translate_modal` reported only the keymap-owned binding after
a counted action, even though the grammar had consumed a decimal prefix before
that binding. The grammar now retains the raw count keystrokes separately from
the saturated semantic count and prepends them to the resolved sequence. The
same sequence is carried through character-taking commands, so completed
feedback describes gestures such as `3 l`, `2 Space r`, and `2 f x` exactly.

`ColonCommand::ALL` omitted the live `Path` identity even though `COMMANDS`
exposed `:path`. The identity inventory and palette registry now agree, with
tests rejecting duplicate identities, missing identities, and duplicate
command spellings. The built-in keymap audits now cover every binding scope,
both the default and fast-pane registries, both modal modes, semantic target
and discovery metadata parity, scoped/global shadowing, dead or executable
namespaces, and exact/prefix ambiguity. No binding was redesigned.

The `reload` command metadata described only file reloads, while
`App::reload_active` also refreshed directory explorers and the Git status,
branches, worktrees, log, and stash lists. The registry description, help,
hints, user guide, and Helix deviation record now describe the same boundary.
`reload_dispatch` makes that routing explicit and testable; Git blame remains
a generated attribution view rather than a refreshable Git list.

Regression coverage is provided by
`colon_command_inventory_matches_the_palette_registry` and
`command_palette_spellings_are_globally_unique` in `src/command.rs`;
`resolved_binding_spelling_keeps_the_typed_count_prefix` in
`src/input_grammar.rs`; `normal_and_select_bind_the_same_sequences`,
`every_entry_point_can_name_itself`,
`no_scoped_binding_shadows_a_global_binding`,
`every_namespace_is_unique_reachable_and_not_an_exact_binding`, and
`built_in_bindings_have_no_exact_prefix_ambiguity` in `src/keymap.rs`;
`reload_hint_describes_every_view_the_dispatcher_refreshes` in
`src/key_hints.rs`; `reload_dispatch_matches_the_file_explorer_and_git_list_contract`,
`command_prompt_filters_and_completes_commands`,
`completed_key_bindings_report_the_typed_sequence_and_action`, and
`counted_colon_binding_echoes_failure_and_retains_its_error_notification` in
`src/app/tests/commands.rs`; and
`a_count_applies_and_the_interaction_line_settles_on_the_completed_action` in
`src/snapshot.rs`.

Known limitation: preserving the existing saturated-count behavior and exact
typed feedback means the grammar continues retaining every accepted raw count
digit after the semantic value reaches `999999`. Imposing a digit bound would
be a separate user-visible count-policy change.

## Report

A focused hardening review was needed for command identity, key-sequence
dispatch, availability, help, hints, and completed-command feedback. The
review was proactive rather than evidence of a previously known defect, so
changes were limited to confirmed problems and the keymap registry remained
the single source of truth.

The primary review surface was `src/command.rs`, `src/keymap.rs`,
`src/key_hints.rs`, `src/help.rs`, command dispatch in `src/app/input.rs`, and
their tests. The audit covered duplicate and unreachable bindings, prefix
ambiguity, counts, fallback and cancel behavior, special-buffer overrides,
mode transitions, availability reasons, command-palette parity, macro
interaction, metadata accuracy, and agreement among execution, hints, help,
and feedback. Command and binding decisions had to preserve the deliberate
Runyte deviations recorded in `context/reference/helix-keymap-v1.md`, while
special-surface behavior had to follow
`context/reference/ui-vocabulary.md`.

The review confirmed that completed-command feedback omitted numeric prefixes:
a gesture such as `3 l` was reported as `l`, and a counted `2 Space r` failure
was reported as `Space r`. The command identity inventory also omitted
`ColonCommand::Path` even though the palette exposed `:path`. Finally, the
registry described `Space r` and `:reload` as file-only or file-and-explorer
operations even though dispatch also refreshed supported Git lists. Existing
registry invariants did not exhaustively cover every scope, semantic modal
parity, namespace reachability, exact/prefix ambiguity, or the selectable
fast-pane keymap.

Every confirmed defect required registry-level and behavior-boundary
regression coverage. The implementation and each distinct fix required an
independent code review, incorporation or technical disposition of every
actionable finding, and re-review after material revisions. Validation
required targeted tests, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` without an
incidental keymap redesign.
