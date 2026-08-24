---
title: "Detached workspace hosts start in the project root instead of the launch directory"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 6c00548
---

## Resolution

Commit `6c00548` (`Preserve detached host launch directories`) fixed
`workspace::lifecycle::start_detached_host`, which used the endpoint's project
root as both the stable workspace identity and the child process's current
directory. Automatic persistent launches and `--wait` therefore initialized
`App::working_directory` at the project root even when the invoking editor was
started below it.

`HostStartup` now carries an optional editor working directory separately from
the endpoint-owned project root. Automatic persistent launches pass their
launch directory, and `--wait` passes its caller directory, while lifecycle
operations that do not continue an editor invocation retain the project-root
default. The lifecycle boundary canonicalizes this requested directory and
requires it to remain inside the endpoint's project root before spawning, so
changing editor-relative path semantics cannot silently change workspace
identity or publish a host at another endpoint.

Coverage is provided by
`detached_host_keeps_the_requested_editor_directory_below_the_project_root` and
`detached_host_rejects_a_working_directory_outside_its_project_before_spawn`
in `tests/persistent_host.rs`. The former exercises the real detached host and
local protocol through `:quit-here`; the latter verifies containment is
enforced before process launch and before endpoint files are published.

## Report

When automatic persistent mode or `--wait` started a detached workspace host
from a directory nested below the project root, the child process was forced to
use the project root as its current directory. Consequently,
`App::working_directory` began at the project root rather than at the directory
from which Runyte was invoked.

This differed from standalone behavior and changed relative opens, path
completion, and `:quit-here`. A detached host needed to retain the caller's
requested working directory while continuing to identify and register the
correct project root.
