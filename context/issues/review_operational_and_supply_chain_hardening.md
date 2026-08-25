# Review operational and supply-chain hardening

Conduct a focused hardening review of build, test, packaging, release, and
dependency boundaries that affect the reliability of the shipped editor. This
is a proactive review rather than evidence of a known defect; make changes only
for confirmed problems and do not perform a release as part of this task. Fix
every confirmed problem that is safely within this category; do not stop after
reporting findings.

The primary scope is `Cargo.toml`, `Cargo.lock`, build and release scripts,
CI configuration, `tests/release_packaging.rs`, performance tests, license and
notice inputs, feature gates, and small cross-cutting unsafe or panic-prone
boundaries not owned by another review. Check MSRV enforcement, locked builds,
crate contents, platform conditionals, debug versus release behavior, enabled
dependency features, duplicated or abandoned dependencies, license coverage,
reproducible generated inputs, ignored test matrices, resource-heavy tests,
fuzz and sanitizer gaps at untrusted-input boundaries, and performance
regression thresholds. Follow `context/reference/releasing.md` for any change
to release mechanics.

Add focused checks or tests for every confirmed defect. Dependency upgrades
must have a specific hardening justification and must not become a general
version-refresh exercise.

For every distinct fix, ask a subagent to perform an independent code review
after the implementation is complete. Give the subagent this issue, the fix
diff, the relevant invariants, and the test results. Address every actionable
finding, or record the technical reason it does not apply; if the response
causes a material revision, review that revision again. Run the targeted tests
and the repository's required `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` checks. Report
back to the user only after the subagent review has been incorporated and all
validation is complete.
