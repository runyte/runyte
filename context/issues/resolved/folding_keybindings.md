---
title: "Syntax folding used an inconsistent shifted keybinding"
status: resolved
reported: 2026-08-24
resolved: 2026-08-24
commit: d272522
---

## Resolution

Commit d272522 (`Rebind syntax folding commands`) corrected
`keymap::built_in_bindings`, which assigned toggle-fold to `Space x f` and
fold-all to the shifted `Space x F`. The registry now assigns fold-all to the
unshifted mnemonic `Space x f`, toggle-fold to the repeated namespace key
`Space x x`, and retains unfold-all on `Space x u`. The old shifted spelling
was removed rather than kept as a compatibility alias so execution, hints,
and generated help expose one consistent folding grammar. The user guide and
Helix keymap deviation register document the same bindings.

`syntax_folding_uses_the_unshifted_namespace_bindings` in `src/keymap.rs`
covers all three bindings in Normal and Select modes and verifies that
`Space x F` is unbound. `folds_share_one_projection_across_snapshot_motion_and_panes`
in `src/app.rs` exercises fold-all through `Space x f` and retains the
pane-local folding regression coverage.

## Report

The syntax folding bindings used `Space x f` to toggle the fold at the cursor,
`Space x F` to fold all syntax regions, and `Space x u` to unfold all syntax
regions.

Folding all regions was expected to use the unshifted mnemonic `Space x f`.
Toggling the fold at the cursor was expected to use the repeated namespace key
`Space x x`. Unfolding all regions was expected to remain on `Space x u`.
