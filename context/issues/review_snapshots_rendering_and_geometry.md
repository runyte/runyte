# Review snapshots, rendering, and geometry

Conduct a focused hardening review of presentation-neutral snapshots and their
Ratatui rendering, including all row and cell geometry. This is a proactive
review rather than evidence of a known defect; make changes only for confirmed
problems and preserve the snapshot boundary between core state and frontends.
Fix every confirmed problem that is safely within this category; do not stop
after reporting findings.

The primary scope is `src/snapshot.rs`, `src/ui.rs`, `src/wrap.rs`,
`src/content_alignment.rs`, `src/row_hints.rs`, relevant presentation code, and
snapshot or geometry tests. Check tiny and zero-sized regions, clipping,
scrolling, viewport bounds, soft wrapping, tabs, wide and combining
characters, folds, selections, carets, overlays, lists and prompts, comparison
alignment, terminal cells, stale snapshot data, hidden versus active panes,
and frame-time or allocation growth. Consult the UI vocabulary reference
before changing surface behavior.

Add semantic snapshot and focused geometry regression tests for every
confirmed defect; avoid broad brittle full-frame snapshots where smaller
invariants suffice.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
