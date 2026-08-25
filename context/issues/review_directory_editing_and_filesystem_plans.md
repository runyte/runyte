# Review directory editing and filesystem plans

Conduct a focused hardening review of editable directory projections and the
confirmed filesystem plans they produce and apply. This is a proactive review
rather than evidence of a known defect; make changes only for confirmed
problems. The deferred `fs_plan_symlink_race` issue remains outside this task
unless the user explicitly authorizes its architectural work. Fix every other
confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is `src/directory_buffer.rs`, `src/directory_listing.rs`,
`src/fs_plan.rs`, `src/path_safety.rs`, the relevant file workflows, and their
tests. Check hidden entry identity, refreshes after external changes, unusual
and non-UTF-8 names, rename and move cycles, overwrite conflicts, project-path
containment, symlinks, trash and permanent deletion, cross-filesystem moves,
partial application, rollback behavior, stale confirmations, cache
invalidation, and consistency between the displayed plan and executed effects.

Add temporary-directory regression tests for every confirmed defect. Do not
weaken confirmation or containment rules to make an edge case pass.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
