---
title: "Git discovery launch failures have no decided retry or retention semantics"
status: resolved
reported: 2026-08-30
resolved: 2026-09-05
commit: 9b2f5d5
---

## Resolution

`9b2f5d5` — `Allow explicit retries after Git repository discovery fails`.

`App::refresh_git` required an attached repository, and the command registry
applied that same requirement before dispatch. A discovery error therefore
removed the only existing refresh action that could have recovered Git for a
long-lived persistent session. Discovery submission errors also left
`discovery_complete` false, describing a request rejected by the service queue
as one still running.

The approved policy is one asynchronous discovery attempt per explicit
`:git-refresh` invocation, also reachable through the existing `Space g r`
binding. There is no automatic retry budget or backoff: ordinary editor input,
maintenance, and persistent-session reattachment never retry discovery. All
discovery failures can be retried explicitly, including a failed launch, a
signal exit, and deterministic permission or configuration failures. The policy
does not require guessing which failures are transient or parsing error text.
`GitError::Unavailable` distinguishes an unavailable provider or failed launch
from a child's exit code in `GitError::Failed`; a signal exit alone cannot
establish whether that child reached Git. The macOS fork-before-exec fix in
`ba2f0a7` is unchanged.

`GitWorkflowState` retains the structured `GitError` while a retry is pending.
A separate refresh capability keeps the recovery command available after a
failure while other Git commands still require a discovered repository. Both
the registry and the workflow refuse a second pending attempt. A completed
failure replaces the retained diagnostic; a successful answer clears it and
either attaches and refreshes the repository or records authoritative absence.
A rejected submission leaves retry available and preserves any original
discovery failure, while its own queue error is reported separately.
`GitCliProvider::discover_with_marker_probe` now routes each of its three
`rev-parse` calls through the existing bounded local-read runner, including its
30-second deadline, cancellation, and output ceiling.

The status line names discovery failure with the retry command, and names the
pending retry while it runs. Command-palette reasons and the new Git discovery
row in `:service-health` retain the last diagnostic during that attempt; health
remains degraded until it answers. Failure notifications remain in the bounded
notification history even after recovery. A persistent session owns all this
state for its host's lifetime, so detach and reattach neither clear it nor
replenish any retry budget. These are Runyte Git commands; no Helix binding or
compatibility behavior changes.

Regression coverage:

- `discovery_failures_require_one_explicit_retry_and_keep_diagnostics_until_answered`
  in `src/app/tests/git_discovery.rs` covers the controlled service boundary,
  distinct launch/exit/signal/permission/cancellation errors, palette and health
  presentation, ordinary input while pending, absence of automatic retries,
  both explicit command routes, duplicate refusal, and successful recovery.
- `repeated_discovery_failure_replaces_last_error_and_absence_ends_retry` in
  `src/app/tests/git_discovery.rs` covers repeated failure, retained notification
  history, and authoritative absence after retry.
- `rejected_discovery_submission_is_retryable_and_keeps_the_original_failure`
  and `discovery_can_be_submitted_after_queue_capacity_returns` in
  `src/app/tests/git_discovery.rs` cover a full queue, a stopped service, and
  capacity becoming available without losing the original diagnostic.
- `synchronous_test_provider_can_recover_failed_discovery` in
  `src/app/tests/git_discovery.rs` keeps the injected synchronous test facade
  consistent with the production workflow.
- `persistent_frontend_reattachment_retains_failed_and_pending_discovery` in
  `src/app/tests/git_discovery.rs` covers the editor's detach and frontend
  attachment hooks in both failed and pending states.
- `discovery_distinguishes_failure_to_launch_from_a_git_exit_or_signal` in
  `src/git/tests/discovery.rs` uses an absent executable and the checked-in
  stand-in to prove structured launch failure, exit code, signal, and recovery
  through the real provider.
- `every_repository_discovery_read_has_a_deadline` in
  `src/git/tests/discovery.rs` stalls each discovery read independently.

Linux validation passed `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and the canonical
`cargo llvm-cov --locked --workspace` run at 91.66% total line
coverage, above the unchanged 89% floor. Native macOS validation was not run.

Known limitation: authoritative repository absence and startup without a Git
provider do not offer rediscovery. A stopped Git service is reported but is not
restarted by this command. Retention is in host memory and does not survive a
host restart, crash, or reboot. The retry policy is deliberately explicit;
signal termination is not treated as proof of a retryable pre-exec failure.

## Report

Repository discovery currently retains a signal-terminated Git child as the
workspace's Git failure. The macOS investigation recorded a child that was
terminated before it replaced the Runyte child image with Git. That process
never inspected the repository, so its exit is not an authoritative statement
that the workspace is not a repository or that repository discovery itself
completed normally.

The capability snapshot already distinguishes authoritative repository
absence from discovery failure, but the lifecycle of a failed discovery is
undecided. Retaining the failure forever avoids an unbounded retry loop and
keeps failures visible. Treating every launch failure as final can also leave
Git features unavailable for the life of a persistent session after a
transient operating-system failure that a later launch would survive.

A product decision is required before implementation:

- whether a child that did not successfully reach Git is retryable;
- which failures prove that distinction without matching error strings;
- whether retry is automatic, command-triggered, or exposed as an explicit
  action;
- the retry budget and backoff, if any;
- how the command palette, status, notifications, and service health represent
  transient failure versus authoritative absence; and
- whether persistent sessions retain the last failure across detach and
  reattach.

Any implementation must keep repository discovery asynchronous and bounded.
It must not retry a deterministic configuration or permission failure into a
busy loop, hide the original structured diagnostic, or make ordinary editor
input wait for Git. Tests need a controlled provider that distinguishes a
child that never launched Git from Git itself returning a failure, then proves
the chosen retry and presentation behavior.

This is deferred because the retry and user-facing state model have not been
approved. The macOS fork-before-exec defect itself was fixed in `ba2f0a7` and
is not reopened by this issue.
