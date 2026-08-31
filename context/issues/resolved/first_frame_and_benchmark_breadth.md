---
title: "First-output latency scaled with document work and comparative benchmarks covered too little"
status: resolved
reported: 2026-08-31
resolved: 2026-08-31
commit: d7fdd63
---

## Resolution

Commit `d7fdd63` (`Decouple first output and idle work`) resolved the startup
and idle behavior. `main::run` acquired the terminal only after
`App::new_in_project_with_targets_and_trace` had synchronously opened and parsed
every startup target, so the first terminal byte inherited document-read and
syntax cost. Standalone startup now enters the terminal first and immediately
draws a stable, document-free `Opening workspace…` presentation. Document text
still appears only in the first complete highlighted editor frame, so the
earlier presentation does not introduce unhighlighted text, reflow, or a later
settle. Startup tracing records terminal entry, the startup presentation,
buffer open, syntax completion, and the editor frame as distinct milestones.

That earlier terminal ownership required a signal-safe startup boundary.
`StartupSignalExit` publishes the saved terminal state before signal handlers
are installed; a signal received during synchronous startup restores termios
and terminal modes through async-signal-safe operations and exits immediately.
The normal event-loop path uses a self-pipe instead of a 25 ms termination poll,
and its handler counter prevents the pipe descriptor from closing or being
reused while a signal write is in flight.

Idle work came from fixed-rate coordination rather than screen redraws.
`file_monitor::run_worker` now sleeps until its next debounce or reconciliation
deadline, `git_monitor::run_worker` blocks until a command or real deadline,
and both handles suppress unchanged registrations. Git invalidations refresh
immediately, while the general host maintenance cadence is one second. A
backpressured Git invalidation blocks the dedicated worker on its bounded output
slot, which guarantees delivery when the host drains it rather than stranding a
retained event on the command wait.

The comparative benchmark breadth landed in prerequisite commits `4a90b3a`
(`Record comparative editor quit time`) and `e44e2cf` (`Repeat idle benchmarks
in isolated environments`), with final correctness work in `d7fdd63`. Quit is
defined from the final force-quit keystroke to successful process exit and is
reported only for complete samples. Idle results use repeated fresh isolated
processes and report median plus range. Linux CPU accounting includes live
descendants and reaped-child ticks. The startup settle heuristic now requires
substantive output, so the short startup presentation cannot masquerade as the
settled editor frame on a slow machine.

Regression coverage is in:

- `tests/local_protocol.rs`:
  `a_blocked_document_open_presents_an_intentional_startup_screen`,
  `termination_during_a_blocked_startup_open_restores_the_terminal`, and
  `termination_signal_restores_the_terminal_and_preserves_its_exit_status`;
- `src/git_monitor.rs`:
  `draining_a_full_output_delivers_the_retained_invalidation` and
  `unchanged_repository_registration_does_not_wake_the_worker_again`;
- `src/file_monitor.rs`:
  `unchanged_registrations_do_not_wake_the_worker_again`;
- `src/startup.rs`:
  `terminal_entry_precedes_document_dependent_startup_work` and
  `report_preserves_phase_order_and_elapsed_values`;
- `benchmarks/test_ptybench.py`:
  `test_loading_presentation_cannot_settle_before_the_editor_frame`,
  `test_proc_stat_includes_reaped_child_ticks`, quit validity, and repeated-idle
  aggregation coverage;
- `benchmarks/test_run.py`: isolated-environment, discovery, completeness, and
  result-format coverage.

Known limitation: CPU percentage remains unavailable without `/proc`, including
on macOS, although repeated screen-write measurement remains portable. The
complete editor frame still waits for synchronous initial document and syntax
work; the startup presentation is deliberately non-interactive. Comparative
quit remains in the matrix even though Neovim leads five rows and Runyte only
ties Helix on `long.lua`. The other candidate comparative categories in the
report remain future benchmark work rather than silently claimed coverage.

## Report

The 2026-08-29 result set in `context/reference/startup-performance.md` shows
Runyte reaching a settled first frame before Neovim and Helix in every row, and
Neovim emitting its first byte earlier than Runyte in five of six.

| Fixture | Size | neovim first / ready | runyte first / ready |
| --- | --- | ---: | ---: |
| `short.txt` | 17 kB | 6 / 18 ms | 5 / 6 ms |
| `medium.txt` | 171 kB | 6 / 17 ms | 6 / 7 ms |
| `long.txt` | 1.7 MB | 6 / 22 ms | 16 / 17 ms |
| `short.lua` | 17 kB | 6 / 30 ms | 10 / 12 ms |
| `medium.lua` | 171 kB | 6 / 46 ms | 28 / 29 ms |
| `long.lua` | 1.7 MB | 6 / 175 ms | 150 / 152 ms |

Neovim's first paint is 6 ms regardless of document size or language. Runyte's
grows with both: 5 ms to 16 ms across the `.txt` sizes, and 10 ms to 150 ms
across the byte-identical `.lua` ones. The difference is not the total work each
editor does, because Runyte settles first everywhere; it is when the first frame
is emitted. Neovim paints before it has finished reading and parsing the
document, while Runyte's first paint waits for more of that work to complete.

The same result set records Runyte at 0.30% idle CPU over a ten-second window
against 0.00% for both other editors, with zero screen writes for all three. That
is one window rather than a median, so it is a point estimate rather than a
confirmed difference.

### Expected direction

First paint should become as close to independent of document size and language
as the terminal allows, without moving the settled frame later. The settled time
is Runyte's current lead and must not be traded for an earlier first byte: a
first paint that is followed by a visible reflow, a flash of unhighlighted text,
or a later settle is a worse result than the current one, not a better one.
Background syntax already exists as the precedent for deferring work off the
first frame; what else the first frame currently waits for needs to be measured
rather than assumed.

Idle cost should reach a repeatable 0.00%, or the work that keeps it above that
should be named and justified. `context/reference/startup-performance.md` must be
consulted and updated when anything that runs on a timer changes.

The objective is to lead in every measured category. A category where Runyte does
not lead is recorded as such and kept in the matrix rather than dropped from it.

### Broader benchmarks

`benchmarks/` measures two things: time to a settled first frame, and idle cost.
`tests/performance.rs` holds in-process budgets for large-document open, redraw,
and typing, but those are CI assertions rather than comparative measurements, so
no cross-editor figure exists for anything but startup and idle.

Candidates for new categories, each of which needs a definition of what is
measured and a check that every editor is doing the same work before a
cross-editor row is claimed:

- keystroke-to-frame latency while typing, at each fixture size, with and
  without a language;
- scrolling and paging through a large document, including a jump to its end;
- incremental reparse latency after an edit;
- literal and regular-expression search in the current buffer and across the
  workspace, measured both to the first result and to completion;
- the file finder and the content finder: time to the first ranked result and to
  a complete scan of a large tree;
- open, save, and reload of a large file;
- soft wrap, very long lines, and minified single-line documents;
- multi-cursor edits at scale, undo, and redo;
- Git gutter and status refresh in a large repository;
- terminal emulation throughput, in bytes of program output per second;
- startup and idle with a language server attached;
- resident memory at rest, with a large document open, and after a long editing
  session;
- quit time, which the pty harness already produces but which no result set
  records.

### Constraints

- Fixtures stay generated from a fixed seed rather than taken from the
  repository, so that a result does not change when Runyte's source does.
- A cross-editor row is only meaningful where each editor does the same work.
  Parser and feature availability must be confirmed per result set and stated,
  as the current document does for tree-sitter.
- The harness must stay usable with `--only runyte` on a machine that has no
  other editor installed.
- Idle measurement reads `/proc` and is Linux-only today. Any new
  resource-measuring category needs a macOS equivalent or an explicitly recorded
  limitation.
- Measurements are taken deliberately and recorded by hand with the machine,
  editor versions, and method; assertions that fail in CI belong in
  `tests/performance.rs`. Absolute values are machine-specific and are not
  comparable across result sets from different hardware.
- `benchmarks/README.md` documents what each fixture isolates and must describe
  any new category in the same terms.
