# Review syntax and background parsing

Conduct a focused hardening review of language detection, Tree-sitter parsing,
incremental edits, highlighting, structural queries, folds, and the background
parse worker. This is a proactive review rather than evidence of a known
defect; make changes only for confirmed problems. Fix every confirmed problem
that is safely within this category; do not stop after reporting findings.

The primary scope is `src/syntax/`, `src/app/syntax_workflows.rs`, syntax-driven
editing callers, and syntax tests. Check byte and character coordinate
conversion, incremental edit construction, revision identity, stale result
rejection, pending-edit translation, cancellation and replacement of queued
work, parser and query failures, malformed trees, injection limits, grammar
detection, large files, deep or adversarial syntax, fold-range validity,
highlight clipping, and behavior while no current tree is available.

Add regression tests for every confirmed defect, using background-worker and
direct-parser coverage as appropriate. Preserve the documented fidelity and
asynchrony limits unless a defect requires narrowing them.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
