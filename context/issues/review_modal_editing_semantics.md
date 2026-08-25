# Review modal editing semantics

Conduct a focused hardening review of Runyte's selection-first editing
semantics. This is a proactive review rather than evidence of a known defect;
make changes only for confirmed problems and preserve deliberate differences
from Helix. Fix every confirmed problem that is safely within this category;
do not stop after reporting findings.

The primary scope is `src/app/editing.rs`, `movement.rs`,
`src/structural_selection.rs`, `wrap.rs`, `table.rs`, `jumplist.rs`, macro and
register handling, and their tests. Check counts, primary and secondary
selections, overlapping ranges, empty lines and documents, document ends,
delete/change/yank/paste symmetry, indentation, joining, case conversion,
smart newline, structural objects, syntax-unavailable fallbacks, visual-line
movement, jumplist updates, macro recording and replay, undo grouping, and
Unicode. Consult the keymap reference before changing any command behavior.

Add behavior-boundary regression tests for every confirmed defect. Do not
reinterpret a documented Runyte keymap deviation as a bug.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
