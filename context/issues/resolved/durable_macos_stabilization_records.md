---
title: "Landed macOS stabilization reviews remained as transient development records"
status: resolved
reported: 2026-08-30
resolved: 2026-08-30
commit: f8f9b13
---

## Resolution

Commit `f8f9b13` (`Remove landed stabilization reviews`) removed
`context/reviews/macos_test_stabilization.md` and
`context/reviews/git_child_sigkill_on_darwin.md` after the reviewed work and
its follow-up corrections shipped in version 0.1.5. The files were correct as
cross-machine, pre-merge records, but keeping them after release left
pre-landing findings beside current source and issue records without stating
which findings were complete.

The retained destinations follow the repository's context boundary. The
macOS temporary-path alias and remaining root-policy work are recorded in
`context/issues/centralized_test_runtime_roots.md`; terminal colour behavior
is current in `context/reference/terminal-compatibility-v1.md`, with the API
hazard in `context/issues/terminal_color_depth_render_boundary.md`; and the
wait-marker, trash-coverage, and Git-fixture findings each have their own open
issue. The asynchronous-CI issue retains the Git fork-before-exec diagnosis,
the reason for `CommandExt::process_group(0)`, and the burn-in evidence. The
separate retry and retention question is isolated in
`context/issues/deferred/git_discovery_launch_failure_semantics.md` because it
still requires a product decision.

The completed behavior remains covered by
`src/git_monitor.rs::linked_worktree_watches_checkout_private_and_shared_metadata`,
`tests/local_protocol.rs::durable_completion_wins_a_race_with_launcher_loss`,
`tests/diagnostic_log.rs::attaching_with_logging_flags_reports_the_retained_configuration`,
`src/workspace/transport.rs::symlinked_owner_wide_inventory_is_refused_without_following_it`,
`src/workspace/transport.rs::boot_namespace_is_stable_and_path_safe`,
`src/git/cli.rs::darwin_parallel_groups_preserve_each_leaders_wait_status`,
`src/ui.rs::exact_rgb_is_preserved_only_for_truecolor_terminals`, and
`src/ui.rs::integrated_terminal_colors_follow_the_outer_terminal_depth`.
This context-only removal was validated with `git diff --check`; it did not
require rebuilding the Rust source.

Known limitation: the six open implementation issues and the deferred
Git-discovery decision extracted from the reviews remain intentionally
unresolved.

## Report

`context/reviews/macos_test_stabilization.md` reviewed the
`fix/macos-tests` branch at `bdb11c1`, and
`context/reviews/git_child_sigkill_on_darwin.md` recorded the investigation
that led to `ba2f0a7`. The reviewed branch, the required follow-up corrections
in `29a660b` and `c156283`, and the Git launch fix all shipped in version
0.1.5.

Files under `context/reviews/` are transient by repository convention. Once a
branch lands, verified diagnoses, deliberate constraints, known limitations,
and named regression tests belong in resolved issue records or current
reference documents. Leaving the reviews in place made pre-merge language
such as “should be corrected before the branch lands” look current after the
release and required readers to determine which findings were implemented.

The completed work required these durable destinations:

- the macOS temporary-path alias diagnosis, semantic Git-monitor barrier,
  wait-client status barrier, client-owned colour adaptation, and their
  regression tests;
- the fork-before-exec diagnosis, the reason for
  `CommandExt::process_group(0)`, the process-group ownership constraints, and
  the Darwin burn-in evidence;
- terminal colour behavior in
  `context/reference/terminal-compatibility-v1.md` as the current source of
  truth; and
- separate open or deferred issues for every unresolved implementation or
  product decision.

The reconciliation must use reachable public commit identifiers and must not
claim that the remaining temporary-root, trash-coverage, marker-suffix,
rendering-API, Git-fixture, or discovery-semantics work shipped in 0.1.5. No
source behavior belongs in the context-only cleanup.
