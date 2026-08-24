---
title: "Persistent workspace mode rejects ordinary file targets"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 22f8807
---

## Resolution

Commit 22f8807 (Preserve targets under persistent workspace defaults) limited automatic persistent-mode selection to bare implicit launches.

The defect was in `main.rs`'s `run` function. After loading configuration, it rewrote every implicit launch to `LaunchMode::Attach` when `workspace.mode` was `persistent`, including launches that carried files, directories, or `+LINE[:COLUMN]` positions. The host-mode validation correctly rejects targets for explicit attach operations, so the automatic rewrite turned an ordinary `runyte note.txt` into an invalid host-management invocation.

The new `uses_automatic_persistent_mode` predicate requires the invocation to have no targets as well as no explicit mode before selecting attach behavior. Target-bearing invocations deliberately remain standalone. This preserves caller-relative path resolution, directory-target behavior, and initial caret positions; the local attach protocol's `OpenBuffers` request carries paths but does not represent the complete `LaunchTarget`, so forwarding only the path would silently discard supported launch semantics. The README now makes the bare-launch boundary explicit.

The test `persistent_default_only_changes_bare_implicit_launches` in `src/main.rs` covers a bare invocation, a file, a directory (`runyte .`), a positioned file, explicit `--standalone`, and the standalone configuration default. `targetless_standalone_launches_open_about_but_paths_keep_their_meaning` in the same file continues to cover the distinction between bare, file, directory, and explicit host-mode launches.

## Report

Priority: P1.

With `workspace.mode: persistent`, an ordinary target-bearing invocation such as `runyte note.txt` was rewritten to `LaunchMode::Attach` in `src/main.rs`. The host-management validation that followed rejected all targets for attach mode, so enabling persistent mode broke normal file-opening invocations.

Automatic persistent-mode selection needed to retain and open the requested file targets through the workspace host, or target-bearing invocations needed to remain standalone. A configuration setting was not expected to make the editor reject a normal file-open command.

Relevant code was `src/main.rs`, in automatic persistent-mode selection and target validation for host-management modes.
