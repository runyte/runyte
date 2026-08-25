# Review buffer persistence and file lifecycle

Conduct a focused hardening review of opening, retaining, reloading, saving,
renaming, sharing, and closing file-backed buffers. This is a proactive review
rather than evidence of a known defect; make changes only for confirmed
problems. Fix every confirmed problem that is safely within this category; do
not stop after reporting findings.

The primary scope is `src/buffer.rs`, `src/app/file_workflows.rs`,
`src/workspace/buffers.rs`, and their direct callers and tests. Check atomic
saves, dirty-state accuracy, external-change detection, save-as and forced-save
semantics, metadata and permission preservation, symlinks, line endings,
partial I/O, path identity, duplicate opens, shared buffers, close protection,
wait-owned buffers, binary detection, and cleanup after failure. Include races
between reading, editing, external replacement, saving, and closing.

Add focused regression tests for every confirmed defect, using temporary
directories and injectable environment-derived paths. Preserve the documented
standalone and persistent-session behavior.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
