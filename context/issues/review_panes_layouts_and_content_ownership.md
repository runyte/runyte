# Review panes, layouts, and content ownership

Conduct a focused hardening review of the pane tree and the ownership and
lifecycle of content shown in panes. This is a proactive review rather than
evidence of a known defect; make changes only for confirmed problems. Fix every
confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is `src/layout.rs`, pane and buffer state in `src/app.rs`,
`src/app/presentation.rs`, file and terminal workflows, `src/diff_view.rs`, and
their tests. Check recursive layout invariants, minimum geometry, active-pane
validity, recency-aware focus, splits and closes, shared buffers, last-pane
rules, terminal ownership, special-buffer retention, comparison pairs,
maximized and Zen modes, resize operations, workspace switching, stale content
identifiers, and cleanup after replacement or failure. Consult the UI
vocabulary reference before changing pane or content semantics.

Add state-transition regression tests for every confirmed defect, including
sequences that combine splits, closes, maximization, and workspace changes.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
