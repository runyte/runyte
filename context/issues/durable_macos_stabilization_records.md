# Landed macOS stabilization reviews remain as transient development records

`context/reviews/macos_test_stabilization.md` reviewed the
`fix/macos-tests` branch at `bdb11c1`, and
`context/reviews/git_child_sigkill_on_darwin.md` recorded the investigation
that led to `ba2f0a7`. The reviewed branch, the required follow-up corrections
in `29a660b` and `c156283`, and the Git launch fix all shipped in version
0.1.5.

Files under `context/reviews/` are transient by repository convention. Once a
branch lands, verified diagnoses, deliberate constraints, known limitations,
and named regression tests belong in resolved issue records or current
reference documents. Leaving the reviews in place makes pre-merge language
such as “should be corrected before the branch lands” look current after the
release and leaves readers to determine which findings were implemented.

The completed work should be reconciled as follows:

- preserve the macOS temporary-path alias diagnosis, semantic Git-monitor
  barrier, wait-client status barrier, client-owned colour adaptation, and
  their regression tests in resolved records or the relevant existing ones;
- preserve the fork-before-exec diagnosis, the reason for
  `CommandExt::process_group(0)`, the process-group ownership constraints, and
  the Darwin burn-in evidence in the asynchronous-CI issue's eventual
  resolution;
- keep terminal colour behavior in
  `context/reference/terminal-compatibility-v1.md` as the current source of
  truth;
- retain unresolved implementation findings as their own open issues and the
  undecided Git-discovery semantics as a deferred issue; and
- remove both review files only after every durable or unresolved statement
  has a destination.

Resolved records must use reachable public commit identifiers and name the
tests that cover each behavior. They must not claim that the remaining
temporary-root, trash-coverage, marker-suffix, rendering-API, Git fixture, or
discovery-semantics work shipped in 0.1.5. No source behavior should change as
part of this context-only reconciliation.
