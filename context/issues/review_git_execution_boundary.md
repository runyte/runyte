# Review Git execution boundary

Conduct a focused hardening review of the boundary that invokes Git and turns
its output into bounded structured results. Treat repository contents, refs,
paths, configuration, hooks, and Git output as untrusted input. This is a
proactive review rather than evidence of a known defect; make changes only for
confirmed problems. Fix every confirmed problem that is safely within this
category; do not stop after reporting findings.

The primary scope is `src/git/cli.rs`, `service.rs`, `patch.rs`,
`repository_lock.rs`, and their provider tests. Check argument construction and
option termination, revision and path ambiguity, hostile names, output and
error bounds, invalid encodings, process and credential-prompt behavior,
environment inheritance, cancellation, timeouts, child cleanup, repository
locking, concurrent operations, patch validation, path containment, and
failure classification. Commands must remain argument vectors and never pass
through a shell.

Add isolated-repository regression tests for every confirmed defect, including
hostile names and subprocess failures where applicable.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
