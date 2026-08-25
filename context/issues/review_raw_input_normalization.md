# Review raw input normalization

Conduct a focused hardening review of raw terminal input decoding and its
normalization into frontend-independent editor input. This is a proactive
review rather than evidence of a known defect; make changes only for confirmed
problems. Fix every confirmed problem that is safely within this category; do
not stop after reporting findings.

The primary scope is `src/tui/input.rs`, `src/input.rs`,
`src/input_grammar.rs`, the event-loop input boundary in `src/main.rs`, local
protocol input DTOs, and their tests. Check modifier and shifted-key handling,
enhanced keyboard events, repeat detection, escape ambiguity, bracketed paste,
large paste bounds, mouse coordinates and gestures, resize events, focus
events, unsupported events, standalone versus attached equivalence, macOS and
Linux differences, and input received during mode, pane, or workspace
transitions. Consult the terminal compatibility reference before changing raw
event behavior.

Add boundary regression tests for every confirmed defect and keep platform
normalization separate from command dispatch policy.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
