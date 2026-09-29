# Directory tree interactions

## Scope

Make the directory tree a directly actionable navigation surface while retaining
its workspace root, lazy listings, and separation from buffers and split panes.

## Implementation

1. Add `editor.directory_tree_width` to YAML and the settings buffer, defaulting
   to 33 columns (previously 28). Clamp live geometry to leave usable pane space.
   Retain mouse/command width overrides while hiding and showing the tree.
2. Integrate the sidebar boundary with horizontal pane focus and resizing.
   Support dragging its right border and directional resize commands with the
   same edge semantics used by ordinary panes.
3. Bind new, delete, move, and rename directly in the tree scope. Tab toggles a
   default-visible, dimmed footer separated from entries by a horizontal rule.
   Include `v: open in v-split` and `s: open in h-split`; title the tree `[dir tree]`.
   Derive legend key spellings from the effective keymap; wrap for narrow widths.
4. Apply new, move, and rename on prompt submission through the existing checked
   filesystem plan and reconciliation path. A trailing `/` creates a directory.
   Delete captures a checked plan and asks `y/N` on the interaction line; Enter,
   Escape, and `n` cancel. Preserve trash-first deletion and collision refusal.
5. Add `v`/`s` file opening in vertical/horizontal splits. With multiple panes,
   Enter, `v`, and `s` capture numbered destinations in screen order, temporarily
   replacing pane titles. Digits select the pane to open in or split; Escape
   cancels. Direct digits use the same ordering. For more than nine destinations,
   accept a numeric prefix and Enter to disambiguate it.
6. Carry footer and pane-title presentation through the private frontend
   protocol, update help and reference documents, and add regression coverage.

## Validation

Cover real input dispatch, immediate filesystem effects and refusal, delete
cancellation, focus transfer, split/destination opening, mouse and command
resizing, settings persistence, and standalone/attached presentation. Run
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
and canonical `cargo llvm-cov --locked --workspace`; retain the 89% floor.
Native macOS and Windows validation remains the responsibility of their CI jobs.

## Completion

Implemented and validated on Linux on 2026-09-29. The tree now applies individual
checked filesystem plans directly; the internal plan builder and open-buffer
reconciliation remain shared with existing file-management behavior. The tree
scope hides global binding continuations beneath its direct keys, including
`m`, consistently in dispatch, validation, help, and hints. Private frontend
protocol version 67 carries the footer and removes obsolete tree staging fields;
pane numbers reuse semantic pane titles.

Regression tests in `src/app/tests/directory_tree.rs` cover direct actions,
collision refusal, open-buffer retargeting, freshly created directory navigation,
trash-first deletion and cancellation, stale delete plans, pane focus, mouse and
command resizing, split destination selection, numbering with missing pane IDs,
two-digit destinations, settings persistence, footer geometry, and wire round
trips. `ui::tests::directory_tree_legend_is_dimmed_below_a_separator` in
`src/ui.rs` covers footer placement, separator, and dimmed styling.

Validation passed:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test` (including integration suites)
- `cargo llvm-cov --locked --workspace`: 136,335 of 148,166 lines covered
  (92.02%), above the unchanged 89% floor.

The complete test and coverage runs required execution outside the sandbox
because its restrictions deny socket timeout operations used by PTY subprocess
fixtures. Native macOS and Windows checks were not run locally.

## Review follow-up

Tree exit now applies terminal-to-document and read-only mode rules. Each action
captures a single checked plan and applies it without borrowing the explorer or
plugin confirmation slot; busy submissions retain their entered value for retry.
There is no retained staging history, pending-count presentation, or pending-tree
quit/host guard. Completed operations publish known entry kinds immediately,
without filesystem reads in `kind()`, and refresh destination siblings after
revealing previously collapsed directories. Superseded listing results cannot
replace those newer observations.

Legend wrapping is cached by registry identity and width; all seven action
labels share one effective-binding traversal when the cache changes. Help uses
registry spelling markers, and shadowed-binding discovery includes global
continuations hidden by direct tree keys. Configuration loading validates the
same 12–240 width range as the settings page.

Regression coverage adds read-only and terminal focus transitions, busy retry
and confirmation ownership, injected-keymap help/legend consistency, width
bounds, immediate-plan preconditions, cached entry metadata, moves outside the
workspace, and refresh races. All required checks passed after these fixes,
including 3,338 library tests plus integration suites and the coverage result
recorded above.
Trash-only deletion and the sidebar's directional edge semantics are retained.
