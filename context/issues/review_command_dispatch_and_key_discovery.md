# Review command dispatch and key discovery

Conduct a focused hardening review of command identity, key sequence dispatch,
availability, help, hints, and completed-command feedback. This is a proactive
review rather than evidence of a known defect; make changes only for confirmed
problems and preserve the registry as the single source of truth. Fix every
confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is `src/command.rs`, `src/keymap.rs`, `src/key_hints.rs`,
`src/help.rs`, command dispatch in `src/app/input.rs`, and their tests. Check
duplicate and unreachable bindings, prefix ambiguity, counts, fallback and
cancel behavior, special-buffer overrides, mode transitions, availability
reasons, command-palette parity, macro interaction, metadata accuracy, and
agreement among execution, hints, help, and feedback. Consult the Helix keymap
reference before changing commands or bindings and the UI vocabulary reference
before changing special-surface behavior.

Add registry-level and behavior-boundary regression tests for every confirmed
defect. Do not make incidental keymap redesigns during hardening.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
