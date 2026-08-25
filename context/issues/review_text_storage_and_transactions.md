# Review text storage and transactions

Conduct a focused hardening review of Runyte's text storage, character-offset,
selection, and transaction foundations. This is a proactive review rather than
evidence of a known defect; make changes only for problems confirmed from the
implementation or tests. Fix every confirmed problem that is safely within
this category; do not stop after reporting findings.

The primary scope is `src/text.rs`, `src/selection.rs`, the transaction and undo
machinery in `src/buffer.rs`, and their direct callers and tests. Check character
versus byte boundaries, normalized selections, transaction ordering and
composition, inversion, rollback, undo and redo grouping, empty documents,
document ends, overlapping multi-selections, integer conversions, Unicode, and
large inputs. Establish the invariants at each public boundary and verify that
all buffer mutation still passes through transactions.

Add focused regression or property-style tests for every confirmed defect. Do
not broaden the work into new editing features or deliberate keymap changes.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
