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

Coverage lives in `src/clipboard.rs` in
`timing_out_a_helper_also_kills_its_descendants` and
`a_successful_parent_with_stuck_output_cleans_up_its_descendant`, in
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
