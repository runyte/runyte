# Git discovery launch failures have no decided retry or retention semantics

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
