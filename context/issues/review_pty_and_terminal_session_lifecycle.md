# Review PTY and terminal-session lifecycle

Conduct a focused hardening review of PTY integration and terminal-session
ownership from spawn through exit. This is a proactive review rather than
evidence of a known defect; make changes only for confirmed problems. Fix every
confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is `src/terminal/pty.rs`, `src/terminal/mod.rs`,
`src/app/terminal_workflows.rs`, persistent-host integration, and terminal
tests. Check argument and working-directory handling, descriptor ownership,
nonblocking I/O, input and output backpressure, resize races, child exit and
reaping, process-group behavior, shutdown escalation, detach and reattach,
session identity, pane replacement, orphan prevention, terminal review mode,
large paste operations, and cleanup on every error path. Consult the terminal
compatibility reference before changing PTY behavior.

Add regression tests for every confirmed defect, separating deterministic
session-state tests from platform-dependent PTY tests where necessary.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
