# Review LSP client and workspace edits

Conduct a focused hardening review of language-server transport, capability
handling, asynchronous results, and document or workspace mutations. Treat all
server messages as untrusted input. This is a proactive review rather than
evidence of a known defect; make changes only for confirmed problems. Fix every
confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is `src/lsp/`, `src/app/language_workflows.rs`, diagnostics,
completion integration, and LSP tests. Check JSON-RPC bounds and correlation,
startup and shutdown, process failure, cancellation, capability gates, stale
responses, document revisions, UTF-8/UTF-16 position conversion, malformed and
out-of-range edits, overlapping edits, atomic multi-document application,
project-path containment, rename and file operations, diagnostics lifetime,
and request or notification backpressure.

Add focused fake-server regression tests for every confirmed defect and extend
the real-server matrix only where the behavior cannot be established without
one of its pinned servers.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
