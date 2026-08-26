---
title: "Build, CI, packaging, release, and dependency boundaries required a focused hardening review"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 17c1d76
---

## Resolution

Commit 17c1d76 (`Harden locked CI and crate packaging`) found that the primary
Linux lint/test gates and the macOS test gate did not pass `--locked`, allowing
those jobs to resolve a graph different from the committed `Cargo.lock`. It
made each gate enforce the lockfile and added job-scoped workflow assertions so
moving or weakening one command is a test failure.

The audit also found that the crate allowlist included `src/app/tests/`, even
though those modules are repository-only tests. The manifest now excludes that
tree. The packaging regression enumerates every shipped non-test source and
syntax query, the embedded logo, configuration, notices, and all documentation
and license provenance inputs, while rejecting development context and both
integration- and in-source test trees. An independent review identified the
in-source leak and gaps in the first two versions of the assertions; both
review revisions were incorporated before approval.

Coverage is in
`tests/release_packaging.rs::published_crate_contains_the_runtime_inputs_and_not_repository_context`
and `tests/release_packaging.rs::ci_enforces_the_committed_dependency_graph`.
Run it with `cargo test --locked --test release_packaging`.

Known limitation: this audit does not implement binary releases; that separate
work remains tracked by `context/issues/ci_release.md`.

## Report

Build, test, packaging, release, and dependency boundaries that affect the
reliability of the shipped editor required a proactive hardening review. The
review covered `Cargo.toml`, `Cargo.lock`, build and release scripts, CI
configuration, `tests/release_packaging.rs`, performance tests, license and
notice inputs, feature gates, and small cross-cutting unsafe or panic-prone
boundaries not owned by another review.

The audit checked MSRV enforcement, locked builds, crate contents, platform
conditionals, debug versus release behavior, enabled dependency features,
duplicated or abandoned dependencies, license coverage, reproducible generated
inputs, ignored test matrices, resource-heavy tests, fuzz and sanitizer gaps at
untrusted-input boundaries, and performance regression thresholds. Changes to
release mechanics were required to follow `context/reference/releasing.md`, and
the review did not perform a release.

Only confirmed problems safely within this category were to be changed.
Dependency upgrades required a specific hardening justification and were not
to become a general version-refresh exercise. Every confirmed defect required
focused checks or tests, an independent post-implementation code review, the
targeted tests, and the repository checks `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.
