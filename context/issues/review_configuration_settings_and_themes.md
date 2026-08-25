# Review configuration, settings, and themes

Conduct a focused hardening review of configuration discovery, YAML parsing and
patching, setting metadata, live previews, persistence, and theme resolution.
This is a proactive review rather than evidence of a known defect; make changes
only for confirmed problems. Fix every confirmed problem that is safely within
this category; do not stop after reporting findings.

The primary scope is `src/config.rs`, `src/config/`, `src/settings.rs`,
`src/app/settings_workflows.rs`, configuration presentation, and their tests.
Check defaults and aliases, unknown and malformed values, numeric bounds,
duplicate or conflicting YAML, lossless patch refusal, atomic writes,
permission and external-change handling, restart-required settings, preview
rollback, custom themes, missing colors and fallback chains, contrast-sensitive
derived colors, configuration-path identity, and isolation from real user
configuration during tests.

Add temporary-file regression tests for every confirmed defect. Preserve
comments, ordering, and unknown YAML fields wherever the documented patching
contract promises to do so.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
