`src/app.rs` has grown to about 40,000 lines. Roughly 23,000 lines are
production code and 17,000 lines are its in-file tests. The main `impl App`
alone spans about 20,000 lines and coordinates Git, persistent workspaces,
panes, rendering preparation, key dispatch, editing, terminals, search,
filesystem views, configuration, and language-server interactions.

Split `app.rs` into smaller files organized around cohesive responsibilities.
The goal is to make the editor easier to navigate and maintain without changing
its behavior. Choose boundaries from ownership and dependencies rather than an
arbitrary line-count limit. Keep `App` as the editor-level coordinator, but
move feature-specific state and behavior into dedicated components where that
produces a real interface; merely distributing one equally coupled `impl App`
across several files is not the complete outcome.

Likely extraction seams include Git-facing editor workflows, persistent
workspace interaction, terminals, search and pickers, directory and file
operations, language-server coordination, view preparation, prompt editing,
and the pure selection or movement helpers near the end of the file. Split the
large unit-test module into behavior-oriented test modules as the corresponding
code moves, while retaining tests that need access to private application
state.

Do this incrementally so each extraction is reviewable and behavior-preserving.
Key dispatch, help, and hints must continue to use the shared keymap registry;
buffer edits must continue to go through transactions; and existing lower-level
module boundaries for Git, LSP, syntax, buffers, selections, diffs, filesystem
plans, and workspace transport must remain intact rather than being duplicated
inside the new application modules.
