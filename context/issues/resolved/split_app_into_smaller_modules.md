---
title: "Application coordination was concentrated in one oversized module"
status: resolved
reported: 2026-08-24
resolved: 2026-08-25
commit: 04c6626
---

## Resolution

Commit `04c6626` (`Split application coordinator into workflow modules`)
resolved this issue. `src/app.rs` had accumulated editor-level coordination
for nearly every feature as one implementation and kept all of its unit tests
in the same file. The lower-level feature boundaries were still sound, but
locating an application workflow or its tests required navigating an unrelated
body of roughly 40,000 lines.

The application is now organized under `src/app/` by cohesive workflow:
editing, input, files, Git, language services, movement, pickers, presentation,
prompts, search, settings, syntax, terminals, and persistent workspaces. The
root `src/app.rs` retains shared application types, `App` state, and startup
coordination. Each production module names its dependencies explicitly. Pure
completion, movement, and prompt-editing operations have narrow helper
interfaces, while `GitWorkflowState` owns the asynchronous Git bookkeeping and
semantic projection state that previously occupied unrelated fields on `App`.
Git process ownership, language-server transport, terminal emulation, buffer
transactions, syntax, diffs, filesystem plans, and workspace transport remain
in their existing lower-level modules.

The former in-file test module is split under `src/app/tests/` into matching
behavior-oriented modules while retaining private application access. The
source-boundary checks concatenate every production application module, so
moving code cannot silently weaken the transaction and input-grammar
invariants. Public inherent `App` method signatures and the complete production
function inventory were preserved. Key dispatch continues through the shared
keymap registry, and all buffer mutations continue through transactions.

Tests covering the preserved boundaries are
`app_delegates_interpretation_state_to_the_input_grammar` and
`production_selection_replacements_are_revision_tracked` in
`src/app/tests/editing_and_buffers.rs`. The complete application behavior suite
is distributed across the modules in `src/app/tests/`; `cargo test` exercises
those tests together with every integration-test binary.

## Report

`src/app.rs` had grown to about 40,000 lines. Roughly 23,000 lines were
production code and 17,000 lines were its in-file tests. The main `impl App`
alone spanned about 20,000 lines and coordinated Git, persistent workspaces,
panes, rendering preparation, key dispatch, editing, terminals, search,
filesystem views, configuration, and language-server interactions.

The application needed smaller files organized around cohesive
responsibilities so the editor would be easier to navigate and maintain
without changing its behavior. Boundaries were to follow ownership and
dependencies rather than an arbitrary line-count limit. `App` was to remain
the editor-level coordinator, with feature-specific state and behavior moved
into dedicated components wherever that produced a real interface; merely
distributing one equally coupled `impl App` across several files was not the
complete outcome.

Likely extraction seams included Git-facing editor workflows, persistent
workspace interaction, terminals, search and pickers, directory and file
operations, language-server coordination, view preparation, prompt editing,
and the pure selection or movement helpers near the end of the file. The large
unit-test module was to be divided into behavior-oriented test modules as the
corresponding code moved, while retaining tests that needed access to private
application state.

The extraction needed to remain reviewable and behavior-preserving. Key
dispatch, help, and hints had to continue using the shared keymap registry;
buffer edits had to continue going through transactions; and the existing
lower-level module boundaries for Git, LSP, syntax, buffers, selections,
diffs, filesystem plans, and workspace transport had to remain intact rather
than being duplicated inside the new application modules.
