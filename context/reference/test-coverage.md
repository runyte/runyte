# Test coverage

This register records Runyte's source-based Rust coverage baseline and the CI
floor derived from it. It measures which instrumented code the ordinary test
suite executes; it does not replace review of test quality, platform behavior,
or uninstrumented external programs.

The canonical command is:

```sh
cargo llvm-cov --locked --workspace
```

## Target measure

The above-95% target applies to the total **Lines** percentage printed by that
canonical command. It is the only current measure that `cargo-llvm-cov` can
enforce directly and identically in a local run and in CI. It is a reported
source-coverage figure, not a claim that more than 95% of production-only Rust
source has run: stable Rust still instruments inline `#[cfg(test)]` modules in
the same source files as production code, and `cargo-llvm-cov` cannot exclude
only those portions of a file.

A custom source parser that subtracts test item ranges would introduce a
second, Rust-syntax- and configuration-sensitive coverage implementation that
the compiler does not verify. Runyte therefore does not use such a figure as a
gate. New behavior coverage should preferentially live in standalone files
under `tests/` or existing source subdirectories named `tests`, which
`cargo-llvm-cov` excludes, so adding a test does not itself make the target
easier. Revisit this decision if stable Rust gains a compiler-owned way to
exclude inline test code from source coverage.

The 95% floor may be enabled only after the canonical command clears it on both
`x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`. Until then, CI keeps a
lower floor that holds on its measured target, and each platform baseline is
recorded separately.

CI uses `cargo-llvm-cov` 0.9.0, publishes the full per-file summary in the job
summary, retains an HTML report as the `rust-coverage-html` artifact for 14
days, and fails below 89% total line coverage. The floor is deliberately below
the observed baseline because conditional Linux and macOS code changes both the
instrumented denominator and the paths available to a run on one platform.

## 2026-09-05 — macOS

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`aarch64-apple-darwin`, macOS 26.6.2, at base commit `61cb882` plus the
filesystem-plan milestone 4 test and documentation changes. The canonical
`cargo llvm-cov --locked --workspace` command passed all 2,918 non-ignored
tests; 33 tests remained ignored by their existing declarations.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 97,567 | 106,514 | 91.60% |
| Functions | 9,082 | 9,826 | 92.43% |
| Regions | 151,475 | 166,283 | 91.09% |

This completes native macOS validation for the
[filesystem-plan data-safety plan](../plans/completed/PLAN_FS_PLAN_DATA_SAFETY.md).
The native tests exercised exclusive rename collisions, resource forks,
extended attributes, ACLs, copy errors, and recovery behavior.
`src/fs_plan/platform.rs` reached 100% line coverage (22 of 22 lines), and
`src/fs_plan/staging.rs` reached 97.18% (172 of 177 lines).
The editor regressions also verify partial reconciliation with unsaved buffers
and multiple recovery locations retained in the notification buffer.

The full suite and coverage ran outside the sandbox so Unix-socket tests
executed without sandbox denials or early returns. Two path-completion tests
were adjusted to fit the platform's longer absolute temporary paths in their
test viewports; production rendering was unchanged. The enforced 89% floor
and README badge remain unchanged.

## 2026-09-04 — Linux

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`x86_64-unknown-linux-gnu`. The ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 94,920 | 103,687 | 91.54% |
| Functions | 8,811 | 9,551 | 92.25% |
| Regions | 147,167 | 161,695 | 91.02% |

The fresh same-tree baseline before this pass was 94,681 of 103,687 lines
(91.31%), 8,798 of 9,551 functions (92.12%), and 146,828 of 161,695 regions
(90.81%). It is higher than the 2026-09-03 Linux record below, and instruments
more lines, because the tree has moved on since that measurement; the
comparison here is against the fresh run rather than the recorded one.

Every test added by this pass lives in a directory named `tests`, so the
instrumented total did not move: covered lines rose by 239, covered functions
by 13, and covered regions by 339, raising total line coverage by 0.23
percentage points.

The retained tests cover every language-server state the service-health report
can describe for the active buffer — a manager attached under a disabled
policy, a configured server nobody has handshaken with, a ready server and
document, a recognized language with no configured server, and a buffer with no
recognized language — and the syntax row before and after the active buffer has
a tree; the terminal entry points that start a session from a place rather than
from a command line, with the directory each one chooses and the refusals for a
directory view, a row that is not a directory, a row with no entry at all, a
program that cannot start, and a session that has already gone; which terminal
a send of buffer text goes to and each reason it cannot go anywhere, the
freezing of a session's output into a read-only page, the rename refusals and
an unusable name, and the terminal list's rows and details; view alignment at
the top, centre and bottom, the horizontal middle measured against the pane's
width, and soft-wrapped alignment and one-screen-row scrolling in both
directions; the settings registry exhaustively, so that every identity writes
and reads back its own configuration field and no other's, a wrong-typed value
is refused naming what the setting expects, an out-of-range integer and an
unresolvable theme are refused, and only the enumerated types offer values to
choose from; the diagnostic-log page with no logger installed, with an
installed file logger whose header and drained records it shows, with a
degraded status carrying no destination, and with a destination that cannot be
read; and directory transfers refused for a row past the end of a listing, a
row that has never been written, and a row whose name no longer matches the
disk, with a pasted row copied again from its original source.

Two of those needed a boundary of their own. Logging status is process-global —
a logger installs once, and the degraded status a failed installation records
then replaces it — so `tests/log_buffer.rs` reaches all four states in order
inside one test in a binary it owns. The settings sweep is exhaustive over
`SettingId::ALL` rather than a sample because a setting wired to the wrong
configuration field reads back whatever that field still holds, so only the
identity nobody happened to test is wrong.

`app/terminal_workflows.rs` gained 70 covered lines, `settings.rs` 49,
`app/settings_workflows.rs` 42, `app/search_history.rs` 30, `main.rs` 16 and
`directory_buffer.rs` 12. Three files ended one or two uncovered lines worse
than the pass's own baseline (`git_monitor.rs`, `terminal/pty.rs`, `ui.rs`),
which is the run-to-run variation in concurrent paths that earlier passes also
recorded; the figures above are the net.

The largest remaining Linux gaps by uncovered lines are `main.rs` (703),
`app/git_workflows.rs` (691), `app/input.rs` (661), `git/cli.rs` (412),
`app/language_workflows.rs` (356), `workspace/transport.rs` (340), `ui.rs`
(334), `workspace/catalog.rs` (316), `syntax/mod.rs` (294) and
`input_grammar.rs` (269).

The enforced floor and the README badge stay at 89%: the macOS baseline has not
been remeasured on this tree, and the floor has to hold on whichever target CI
measures. The above-95% target has not been reached.

## 2026-09-03 — Linux

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`x86_64-unknown-linux-gnu`. The ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 92,757 | 101,629 | 91.27% |
| Functions | 8,635 | 9,378 | 92.08% |
| Regions | 143,837 | 158,476 | 90.76% |

The table above is the result after the third pass recorded below. The first
pass of the day reached 92,357 of 101,629 lines (90.88%), 8,606 of 9,378
functions (91.77%), and 143,119 of 158,476 regions (90.31%); the second
reached 92,621 of 101,629 lines (91.14%), 8,625 of 9,378 functions (91.97%),
and 143,582 of 158,476 regions (90.60%).

The fresh same-tree baseline before that first pass was 92,124 of 101,629 lines
(90.65%), 8,589 of 9,378 functions (91.59%), and 142,834 of 158,476 regions
(90.13%). It is higher than the 2026-09-02 Linux record below because the tree
has moved on since that measurement, so the comparison here is against the
fresh run rather than the recorded one. Every test added by that pass lives in
a directory named `tests`, so the instrumented total did not move at all:
covered lines rose by 233, covered functions by 17, and covered regions by 285,
raising total line coverage by 0.23 percentage points.

Its retained tests cover Markdown's lexical delimiter fallback for pairs that
open and close with the same character, including escaped delimiters and an
escaped backslash before one; the insert-mode `delete-to-line-end` command
across several carets; the path popup's overlay snapshots, its Ctrl-c
dismissal, a system clipboard that refuses the copy, and a copy into a named
register; report scrolling and result-list paging at both ends; the word each
completion kind is displayed with, including the kinds Runyte has no word for;
an asynchronous Git discard that stops at a refused path and reports what it
had already restored; a worktree started from a remote branch that several
local branches track; every named delimiter pair's own inside and around
commands, with backticks pinned in JavaScript because Rust has no syntax for
them; the command palette's line editing and movement between its suggestions;
and the editing of a typed branch-switch confirmation before it matches.

Review corrected the refused-clipboard test, which had asserted on the
transient action feedback a refusal does not set rather than on the status and
retained notification it does.

The continuation pass began from a fresh same-tree run of that tree: 92,359 of
101,629 lines (90.88%), 8,603 of 9,378 functions (91.74%), and 143,125 of
158,476 regions (90.31%). Its tests also live only in directories named
`tests`, so the instrumented total again did not move: covered lines rose by
262, covered functions by 22, and covered regions by 457, raising total line
coverage by 0.26 percentage points to the table above.

That pass added a standalone `tests/terminal_sequences.rs` that feeds fixed
control sequences straight to the emulator: cursor addressing and its screen
and scroll-region bounds, origin-mode addressing and the cursor report, tab
stops and their clearing, character and line insertion, deletion, erasure and
downward scrolling, the saved cursor in both its escape and CSI forms, insert
and autowrap modes, each graphic rendition and its own reset, and the device
attribute and status answers. It is listed in
`terminal-compatibility-v1.md` as part of the reproducible boundary. The
remaining tests cover the word each LSP symbol kind is displayed with,
including a kind Runyte has no word for; a flat workspace-symbol response,
whose container is kept and whose non-file URI is dropped; one ambient Git
snapshot refreshing an open branch list, the staged index view and a per-file
diff in place, with the index heading counting the staged files the snapshot
reported and naming both sides of a rename; the release of a selected-line
partial-stage guard down each of its three endings — a failed preparation and
a stale answer invalidate it, a stage that lands releases it still valid; and
a paste reaching an open finder's query and a filterable list's filter rather
than the buffer behind them.

Six files ended with one or two more uncovered lines than the baseline run
(`file_picker.rs`, `git/service.rs`, `git_monitor.rs`, `main.rs`,
`terminal/pty.rs`, `workspace/lifecycle.rs`); that is the same run-to-run
variation in concurrent paths recorded for earlier passes, and the totals above
are the net.

The third pass began from the second's clean result and reached the table
above: covered lines rose by 136, covered functions by 10, and covered regions
by 255, for a further 0.13 percentage points on an unchanged denominator.

It covers the page and window motions, which are the family measured in screen
rows rather than in document lines and were unexercised in both projections;
every prompt's own prefix, so no two surfaces that share the interaction line
read alike; `:diff-disk`'s refusals for a scratch buffer, a buffer already
being compared, a file that has gone, and a disk version that is no longer
text; a current-line blame's status answer and its refusal of an unattributed
line; a commit search's refusal outside a repository, its asynchronous request,
and the title a page that hit its own ceiling carries; the finder preview pane
drawn for a text file, a directory, a binary file and a file that vanished
between the listing and the read, in both renderers; the buffer-action menu's
title and rows; and terminal review's painting of its matches, its active
match, and a selection, asserted on drawn cell colours because none of it
reaches the plain text the other terminal tests read.

`ui::draw_buffer_actions` was left uncovered deliberately. It runs only when a
file picker and a buffer-action menu are open at once, and the input layer
never produces that pair: the buffer picker is a filterable list, so its action
menu is drawn through the shared overlay snapshot instead. The 32 lines are
retained rather than removed, in line with the rule against changing production
code to shrink the denominator.

Three files ended one to five uncovered lines worse than the pass's own
baseline (`git/cli.rs`, `workspace/host.rs`, `workspace/transport.rs`): the
same run-to-run variation in concurrent paths recorded above.

The largest remaining Linux gaps by uncovered lines are
`app/git_workflows.rs` (712), `main.rs` (711), `app/input.rs` (662),
`git/cli.rs` (417), `app/language_workflows.rs` (347),
`workspace/transport.rs` (341), `ui.rs` (325), `workspace/catalog.rs` (316),
`syntax/mod.rs` (294), and `input_grammar.rs` (269).

The macOS baseline was refreshed after this pass and is recorded below; the
enforced floor and the README badge move from 86% to 89% with it. The 95%
target has not been reached.

## 2026-09-03 — macOS

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`aarch64-apple-darwin` in GitHub Actions run 173, at commit `d2948d9`. The job
verified the host target before measuring, and the ordinary non-ignored
workspace tests passed under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 92,918 | 101,875 | 91.21% |
| Functions | 8,647 | 9,397 | 92.02% |
| Regions | 144,039 | 158,841 | 90.68% |

This supersedes the 2026-09-02 macOS baseline below, which predated the three
test-only passes recorded in the Linux section above and was the number the
86% floor's headroom had been justified against.

macOS covers more lines than Linux and instruments more of them: 92,918 of
101,875 against 92,757 of 101,629. Its total line percentage is nevertheless
the lower of the two, 91.21% against 91.27%, so it is the target that sets the
headroom. The two now differ by 0.06 percentage points, against 0.37 when both
were last measured on one tree.

The enforced floor is raised from 86% to 89%. That leaves 2.21 percentage
points below the lower measured platform — still several times the observed
divergence between the two targets and the run-to-run variation within one of
them — while turning a material regression red far sooner than 86% did. The
README badge states the floor and changes with it.

The above-95% target has not been reached on either target. A 91% floor is not
taken: it would leave 0.21 percentage points on macOS, less than one ordinary
feature landing with its own untested platform-conditional arm.

## 2026-09-02 — macOS

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`aarch64-apple-darwin` in GitHub Actions run 153. The job verified the host
target before measuring, and the ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 90,481 | 100,380 | 90.14% |
| Functions | 8,450 | 9,253 | 91.32% |
| Regions | 140,408 | 156,605 | 89.66% |

This is the first macOS measurement after the behavior-focused coverage pass.
It supersedes the 2026-08-30 macOS baseline for current floor decisions and
confirms that total line coverage exceeds 90% on both first-class targets. The
enforced floor is raised from 83% to 86%: this turns a material regression red
while retaining 4.14 percentage points of headroom below the lower measured
platform. The above-95% target has not been reached, and 90.14% leaves too
little platform headroom to make 90% a useful regression gate.

## 2026-09-02 — Linux

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`x86_64-unknown-linux-gnu`. The ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 90,688 | 100,194 | 90.51% |
| Functions | 8,456 | 9,238 | 91.53% |
| Regions | 140,743 | 156,351 | 90.02% |

The fresh same-tree baseline before the continuation pass was 90,308 of
100,134 lines (90.19%), 8,438 of 9,234 functions (91.38%), and 140,184 of
156,240 regions (89.72%). The earlier recorded line baseline covered seven
fewer lines because concurrent paths varied between instrumented runs; the
before-and-after comparison here uses the fresh run. Covered lines increased
by 285 while the instrumented total increased by 60, leaving 225 fewer
uncovered lines and raising total line coverage by 0.23 percentage points.

The added tests cover Git cancellation outcomes, unborn-branch pull and rebase
refusals, untracked-branch push remote selection, language-server launch and
JSON-RPC failures, diagnostic clearing, notification filtering, malformed
workspace-edit refusal, repeated local-protocol handshakes, partial and final
wait completion, and standalone path-completion and command-path rendering.
Review replaced fixed-delay LSP assertions with event or wire-message barriers,
made the failed-launch test incapable of executing a stale temporary file,
strengthened file-versus-directory UI assertions, and removed a Git outcome
matrix that enumerated variants without establishing distinct behavior.

The next continuation pass began from a fresh same-tree result of 90,579 of
100,194 lines (90.40%), 8,449 of 9,238 functions (91.46%), and 140,616 of
156,351 regions (89.94%). The clean canonical result above adds 109 covered
lines without changing the denominator, adds seven covered functions and 127
covered regions, and raises line coverage by 0.11 percentage points.

The retained tests cover asynchronous Git log paging, file-diff creation, and
one-shot stash-list creation through service responses; recovery after an LSP
response with the wrong semantic shape; starting-server and relative-document
request refusals before the wire; a reload confirmation overtaken by a host
buffer close; a malformed non-numeric Git history count; and cross-request
wait-buffer isolation without losing either pending request. Review removed a
synthetic repeat-attachment Git test that added no production coverage, routed
the reload race through the host-close boundary, added deterministic LSP wire
barriers and response-token assertions, strengthened the expected Git service
operation checks, and narrowed test names that claimed more than they proved.
The wait-ownership test is retained despite no line-count gain because the
already-covered `ensure!` line hides its newly exercised false condition; its
end-to-end state-integrity assertion is distinct from ordinary wait completion.

The preceding behavior-focused pass had raised its own fresh same-tree Linux
baseline from 89,267 of 99,906 lines (89.35%) to the recorded 90,301 of
100,134 lines (90.18%), a gain of 0.83 percentage points. Its tests exercised
observable refusal, recovery, lifecycle, protocol, picker, Git, LSP, terminal,
and persistent-workspace behavior; coverage-only command and provider sweeps
were removed during review rather than retained for their reported gain.

The enforced floor is 86%. The 95% target has not been reached. The latest
macOS measurement above reached 90.14%, leaving too little headroom for a 90%
cross-platform floor. No post-change macOS measurement exists yet, so the lower
measured platform still provides 4.14 percentage points of headroom above the
floor; neither the floor nor the README badge changes.

## 2026-09-01

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`x86_64-unknown-linux-gnu`. The ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 85,649 | 96,014 | 89.20% |
| Functions | 8,085 | 8,902 | 90.82% |
| Regions | 133,747 | 150,380 | 88.94% |

Against a same-toolchain Linux run immediately before the added tests, the
line denominator stayed at 96,014 while covered lines rose by 385, from 85,264
to 85,649 (88.80% to 89.20%). Covered regions rose by 603. The largest direct
line gains were in `app/input.rs` (+163), `protocol/input.rs` (+69, reaching
100%), and `app/git_workflows.rs` (+58); behavior reached through those
boundaries also covered lines in prompt editing, the finder, file picker, host,
transport, and event-loop coordination.

The enforced floor remains 83%. The target has not yet been reached, and the
macOS baseline below predates these tests; a current macOS run is required
before any cross-platform floor increase.

## 2026-08-30

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`aarch64-apple-darwin`. The ordinary non-ignored workspace tests passed under
instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 78,625 | 93,004 | 84.54% |
| Functions | 7,467 | 8,604 | 86.79% |
| Regions | 123,133 | 145,814 | 84.45% |

The enforced line floor begins at 83%. A later baseline should record the tool,
toolchain, target, covered and total counts, and the reason for changing the
floor.

The README coverage badge states this floor rather than a measured
percentage, so changing the floor means editing the badge in the same
commit.

### Interpretation

`cargo-llvm-cov` excludes standalone files under directories named `tests`, but
stable Rust cannot yet mark every inline `#[cfg(test)]` module as excluded from
coverage. Some inline test code is therefore part of both the instrumented
denominator and the covered count. This baseline is useful for regression
detection within the same setup, but its percentage should not be read as the
share of production-only source that tests execute.
