# Review persistent host and workspace state

Conduct a focused hardening review of persistent-session host ownership and
retained workspace state. This is a proactive review rather than evidence of a
known defect; make changes only for confirmed problems. Fix every confirmed
problem that is safely within this category; do not stop after reporting
findings.

The primary scope is `src/workspace/host.rs`, `catalog.rs`, `lifecycle.rs`,
`service.rs`, `identity.rs`, relevant application workflows, and
`tests/persistent_host.rs`. Check host discovery and launch races, endpoint and
lock ownership, stale registrations, catalog corruption, canonical workspace
identity, attachment exclusivity, control connections, detach and reattach,
workspace switching, protected-state checks, idle retirement, `--wait`
ownership, failure recovery, forced shutdown, cleanup, and isolation between
workspaces. Preserve the documented boundary that persistent sessions do not
promise survival across host or machine failure.

Add regression tests for every confirmed defect with isolated temporary state
roots; tests must not write runtime state into the repository or user paths.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
