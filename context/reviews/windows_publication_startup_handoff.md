# Windows publication startup validation

Prepared 2026-09-25 against `dev` base `e6015827ca6034cf1449c7e728b47271fcdab843`
and the accompanying CI-fix changes. Use the commit containing this handoff,
including its new tests. The [CI investigation](ci_36131655861.md) records the
observations from run `36131655861`.

## 2026-09-25 native acceptance checkpoint

Source commit `be17cc7` (`Fix release details and bound CI startup contention`)
passed all 18 jobs in
[CI run 36136546810](https://github.com/runyte/runyte/actions/runs/36136546810).
The native job used Windows Server 2025, image `windows-2025-vs2026`
`20260922.246.2`. Its ordinary suite passed all five startup-preparation
regressions and the unchanged exact-publication stale-row acceptance. The
required restart/save, Python context adapter, private and public context
bridge, three isolated clipboard, and real rust-analyzer acceptance gates all
ran and passed. Linux line coverage was 91.97% and macOS 91.89%, above the
unchanged 89% floor. Windows coverage remains provisional.

Local Windows 11 build 26200 validation uses Rust 1.97.1,
`x86_64-pc-windows-msvc`, one Cargo build job, two test threads and temporary
configuration/context storage. Formatting, denied-warning all-target Clippy and
`cargo test --locked --workspace --no-fail-fast` pass. All five focused startup
tests, the exact stale-row test, the complete public-attachment target, both
exact restart cases and the exact save case pass.
The save case observed 128 durable saves, 165,618 complete concurrent reads,
1,280 missing-file errors (2), 1,583 sharing violations (32), and exact bytes
after every save acknowledgement. Independent source review found no actionable
findings; no further Rust correction was needed.

The local Python adapter suite passes nine active tests, including all five
required CI cases; 40 platform or separately invoked fixture cases are skipped.
The exact private/public context bridge and real rust-analyzer cases also pass.
Python 3.13.15 uses the existing embedded runtime with a temporary package-path
entry, restored afterward. PowerShell 5.1 wraps stderr in the saved Python log;
the five required passing results were verified with multiline-aware matching.
All three explicit local clipboard cases refuse private-window-station creation
with Windows error 5 because the normal token lacks the required administrator
privilege. They never reach the person's clipboard. The successful exact CI
cases above provide clipboard acceptance; no local clipboard pass is claimed.

PowerShell `Compress-Archive` and `Expand-Archive` preserve the documentation
payload: the extracted user guide's `../contrib/runyte.ps1` link resolves, and
the helper's SHA-256 matches the source. This validates the two-line packaging
change, not a release-mode executable or published archive.

This checkpoint closes the native compilation and acceptance gap recorded
below. The deterministic reader-lock test reproduces the supported contention
path; the exact lock responsible for the original CI failure remains unknown.
The original Node scheduling delay is also unproven, although both current
plugin-conformance jobs pass. A version bump, publication and tagged Windows
archive/checksum validation remain separate release work.

## Failure and implemented correction

`native_manager_visits_selected_live_publication_and_refuses_stale_row` failed
when `public_manager_visit_fixture` started the replacement host. The old host
had exited, but replacement preparation returned `workspace publication lock
is busy` before testing stale-row refusal. Catalog reads can hold the same
name-store lock that preparation needs, even when the stored name is absent.
The old diagnostic does not establish which lock actually caused this run's
failure. Native reproduction and validation remain outstanding.

`src/workspace/windows_endpoint/names.rs::prepare_named_until` retries only
typed `LockFileEx` contention during preparation. Each attempt releases partial
guards before a 25 ms wait and reacquires/revalidates everything. The caller
retains the project lease. Invalid identity, occupied publication, permission
errors and conflicting project ownership retain immediate refusal. Publishing
and binding are outside the retry loop, so uncertain mutations are not replayed.

`src/windows_host.rs` gives preparation a five-second deadline from host-entry,
using the existing detached-readiness budget. The detached parent retains its
own deadline and is never extended. Termination and foreground-parent exit
cancel preparation through the existing startup cleanup owner. Low-level locks
still fail immediately; errors now distinguish project ownership, publication
identity, publication registry and name store.

## Native commands

Use an MSVC Rust toolchain and fixture-owned configuration storage, with CI's
resource envelope. Run from the checkout in PowerShell. Check each exit code;
zero selected tests is not a pass.

```powershell
$env:CARGO_BUILD_JOBS = '1'
$env:RUST_TEST_THREADS = '2'
$env:XDG_CONFIG_HOME = Join-Path ([IO.Path]::GetTempPath()) ('runyte-ci-validation-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $env:XDG_CONFIG_HOME | Out-Null

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --lib workspace::windows_endpoint::names::tests::startup_preparation_ -- --nocapture
cargo test --locked --test windows_public_attachment native_manager_visits_selected_live_publication_and_refuses_stale_row -- --exact --nocapture
cargo test --locked --test windows_public_attachment
cargo test --locked --test windows_session_cli restart_success_replaces_host_and_preserves_protected_state -- --exact --show-output
cargo test --locked --test windows_session_cli restart_refuses_detached_policy_without_breakaway_job -- --exact --show-output
cargo test --locked --test windows_save_visibility successful_saves_have_complete_contents_after_acknowledgement -- --exact --show-output
cargo test --locked --no-fail-fast
```

The focused unit command must run five tests from
`src/workspace/windows_endpoint/names/tests.rs`:

- `startup_preparation_waits_for_stopped_name_reader_and_releases_partial_guards`:
  a channel barrier retains a real stopped-name reader's lock; polling the
  preparation future proves it is waiting. Identity/registry locks are available,
  a competing project lease is refused, and releasing the reader permits publication.
- `startup_preparation_times_out_with_lock_role_without_publishing`: persistent
  name-store contention exhausts the deadline with a role-specific error and
  leaves admission usable afterward.
- `startup_preparation_cancellation_releases_guards_for_each_lock_role`:
  identity, registry and name-store contention all permit cancellation without
  leaving partial guards or publication behind.
- `startup_preparation_revalidates_vacancy_after_contention`: a publication
  installed between attempts is refused and preserved.
- `startup_preparation_revalidates_project_identity_after_contention`: a
  replaced project directory is refused without publication.

Retain the stale-row test's exact-incarnation checks. Do not substitute a sleep,
looser assertion, test-only retry or successful policy refusal for a successful
replacement. If the failure persists, record the new lock role, startup phase
and bounded failure evidence before expanding the retry boundary. A failure
after publication begins must be investigated independently.

Complete the Native Windows CI acceptance steps in `.github/workflows/ci.yml`,
including context bridge, isolated clipboard and real rust-analyzer acceptance.
The failing run skipped language-server acceptance. Clipboard fixtures require
the documented privileges; they must not use the person's clipboard as fallback.
The release workflow's two-line Windows ZIP change also needs PowerShell archive
validation: the extracted package must contain `contrib/runyte.ps1` beside the
documented relative link.

## Linux validation boundary

The ten Python reader tests and fourteen real Node tests pass. Rust formatting,
Linux Clippy and the full Linux `cargo test` suite pass. The suite required
execution outside the sandbox: socket admission was denied inside it, and
that attempt was interrupted after failures and a stalled fixture. Canonical
`cargo llvm-cov --locked --workspace` passes with 91.98% total line coverage
on Linux, above the unchanged 89% floor. Linux
excludes the Windows code and cannot establish
native lock, named-pipe, process or cancellation behavior. The GNU Windows
cross-check stops before compiling Runyte because `x86_64-w64-mingw32-gcc` is
not installed; this is not Windows build evidence. Native compilation and tests
are required before treating failure 2 as resolved or approving the release.
