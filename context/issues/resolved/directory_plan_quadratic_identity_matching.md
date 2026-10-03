---
title: "Directory plan identity matching repeats full scans"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: a6c5ad5
---

## Resolution

Commit `a6c5ad5` (`perf(explorer): index identities when building directory plans`).

DirectorySnapshot::entry now uses its private immutable dense identity table directly. FsPlan groups desired rows once by identity and builds a set of original paths once, removing repeated snapshot and desired-row scans while retaining original-path primary preference, first-row fallback, copy ordering, deletion, and stale-identity rejection.

Coverage: indexed_planning_preserves_original_and_first_duplicate_primary_rows and unchanged_reordered_large_directory_plans_no_operations in src/fs_plan/tests/indexed_planning.rs, alongside 28 passing fs_plan tests. The first applies a mixed rename/copy/delete plan and verifies disk contents; the second covers 2,048 reordered unchanged entries. benchmarks/directory_plan.py compares actual optimized baseline/current source, excluding snapshot reads and input cloning. At 16,384 files, three-sample median planning cost was 177.471 ms before and 10.825 ms after; this is isolated planning cost, not interactive latency.

## Report

Building a filesystem plan from an editable directory projection repeatedly
scans the entire snapshot and desired row list. `FsPlan::build_with_limits`
looks up every desired identity with the linear `DirectorySnapshot::entry`,
then filters the complete desired list once per original snapshot entry.
Even an unchanged directory therefore requires quadratic identity matching
before any filesystem operation is planned. The ordinary editable explorer
does not impose a small entry limit, so large directories can stall the
editor while saving or previewing their changes.

Index the snapshot identities and group desired rows by identity once while
retaining the current order-dependent rules: an unchanged original pathname
is preferred as the primary row, otherwise the first occurrence is primary;
remaining occurrences are copies. Unknown identities must still fail, omitted
identities must still produce deletion, and duplicate target rejection and
source validation must remain intact.

A reproduction builds a no-change plan for snapshots with increasing numbers
of ordinary files and measures only planning after the snapshot is read.
Regression coverage should also exercise reordered rows, duplicate identities,
renames, deletions and stale identities. Keep timing assertions out of tests.
