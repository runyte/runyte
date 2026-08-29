---
title: "Startup and OS integration left process and handoff failure paths unsafe"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 7684a22
---

## Resolution

Commit 7684a22 (`Harden startup and process shutdown`) corrected four
independent lifecycle defects. `main` now installs handlers before terminal
raw mode or durable wait creation, restores terminal state before returning
Unix signal exit status, gracefully shuts down persistent hosts, and cancels
an interrupted `--wait` request after its token becomes known.

Clipboard helpers now run in private process groups and clean up descendants
on timeout, I/O failure, or unsuccessful exit without prematurely killing a
successful helper's legitimate output owner. External openers likewise run in
separate process groups and have their direct children asynchronously reaped.
The cwd handoff now uses a same-directory, exclusively created, mode-0600
temporary file with flush, sync, atomic rename, collision retry, and
owned-candidate cleanup.

A later fix corrected how clipboard cleanup identified the group it was
retiring. `wait_until` reported the helper's exit through `Child::try_wait`,
which reaps, and cleanup then sent `SIGKILL` to `-child.id()`. The kernel
recycles a PID once its group is empty and its leader reaped, so that
negative number could by then name an unrelated process group — a signal
delivered successfully, to a stranger, with the damage appearing wherever
that stranger happened to be. Completion is now observed through
`waitid(WNOWAIT)`, which reports the same status while leaving the helper
unreaped, so the PID and the private process group stay reserved for as long
as cleanup may address them; the leader is collected afterwards, on the
successful path too. Cleanup states which proof it holds — a running leader
or a completed but uncollected one — through `src/process_group.rs`, and
sends nothing when it holds neither. Descendant cleanup is unchanged in
effect: retiring the group after the leader exits is exactly what the
unreaped anchor preserves.

Coverage lives in `src/clipboard.rs` in
`timing_out_a_helper_also_kills_its_descendants`,
`a_successful_parent_with_stuck_output_cleans_up_its_descendant`,
`completed_helper_cleanup_signals_no_recycled_group`, and
`a_completed_but_unreaped_helper_still_owns_its_group`, in
`src/external_open.rs` in
`launched_program_has_a_process_group_separate_from_the_editor`, in
`src/main.rs` in `cwd_file_retry_preserves_colliding_temporary_file` and
`cwd_file_supports_a_near_name_max_target`, and in `tests/local_protocol.rs`
in `termination_signal_restores_the_terminal_and_preserves_its_exit_status`
and `signalling_a_wait_client_cancels_its_durable_request`.

## Report

Process startup, command-line parsing, project-root selection, event-loop
shutdown, terminal restoration, and direct operating-system integrations
required a focused hardening review. The scope included `src/main.rs`,
`src/launch.rs`, `src/startup.rs`, `src/project_root.rs`, `src/clipboard.rs`,
`src/external_open.rs`, cwd-file handoff, and their tests.

The review covered conflicting and ambiguous arguments, relative paths,
project confirmation, standalone versus persistent routing, signals and
panics, alternate-screen and raw-mode restoration, exit codes, `--wait`, cwd
handoff atomicity, clipboard timeouts and child cleanup, external-program
selection and argument handling, cache isolation, detached spawning, missing
environment variables, and Linux/macOS differences. Tests were required to
keep personal configuration, clipboard caches, and program-choice caches out
of scope.
