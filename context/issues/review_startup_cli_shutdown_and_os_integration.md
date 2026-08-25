# Review startup, CLI, shutdown, and OS integration

Conduct a focused hardening review of process startup, command-line parsing,
project-root selection, event-loop shutdown, terminal restoration, and direct
operating-system integrations. This is a proactive review rather than evidence
of a known defect; make changes only for confirmed problems. Fix every
confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is `src/main.rs`, `src/launch.rs`, `src/startup.rs`,
`src/project_root.rs`, `src/clipboard.rs`, `src/external_open.rs`, cwd-file
handoff, and their tests. Check conflicting and ambiguous arguments, relative
paths, project confirmation, standalone versus persistent routing, signals and
panics, alternate-screen and raw-mode restoration, exit codes, `--wait`, cwd
handoff atomicity, clipboard timeouts and child cleanup, external-program
selection and argument handling, cache isolation, detached spawning, missing
environment variables, and Linux/macOS differences.

Add regression tests for every confirmed defect with injectable OS paths and
process adapters where needed. Tests must not touch a person's configuration,
clipboard cache, or program-choice cache.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
