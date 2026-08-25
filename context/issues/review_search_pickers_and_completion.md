# Review search, pickers, and completion

Conduct a focused hardening review of buffer search, project discovery, fuzzy
matching, filterable pickers, previews, path completion, word completion, and
jump labels. This is a proactive review rather than evidence of a known
defect; make changes only for confirmed problems. Fix every confirmed problem
that is safely within this category; do not stop after reporting findings.

The primary scope is `src/finder.rs`, `src/file_picker.rs`, `src/picker.rs`,
`src/word_index.rs`, `src/jump_labels.rs`, search history and picker workflows,
completion support, and their tests. Check cancellation and stale asynchronous
results, result ordering, selection stability, Unicode and case behavior,
literal versus regex behavior, zero-width matches, hidden and ignored files,
symlinks, binary and unreadable files, large repositories, preview bounds,
directory-listing cache invalidation, memory growth, and changes to the active
buffer while results are open.

Add deterministic regression and performance-boundary tests for every
confirmed defect. Preserve the documented differences in Runyte search
semantics.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
