# Review Git projections and workflows

Conduct a focused hardening review of Runyte's Git state models, generated
views, refresh behavior, and user-facing Git mutations. The lower-level Git
execution boundary has its own review. This is a proactive review rather than
evidence of a known defect; make changes only for confirmed problems. Fix every
confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is the Git modules other than the execution-boundary focus,
`src/diff.rs`, Git application workflows, and Git tests. Check status and
gutter consistency, staged-text caching, refresh races, stale projections,
file, hunk, and selected-line staging, patch correspondence, aligned diffs,
renames, conflicts, branch switching, worktrees, history, blame, stashes,
commit, pull and push state, operation availability, repository changes made
outside Runyte, and recovery after failed mutations.

Add isolated-repository regression tests for every confirmed defect. Verify
that all projections converge on actual Git state after both success and
failure.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
