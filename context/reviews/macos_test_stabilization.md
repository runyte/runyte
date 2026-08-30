# Review of the macOS test stabilization branch

Reviewed `fix/macos-tests` at `bdb11c1`, three commits on `b09c444`:
`fef652a` (Stabilize tests on macOS), `cedf0c2` (Adapt colors to terminal
capabilities), and `bdb11c1` (Stabilize wait lifecycle tests).

Verified on Linux at `bdb11c1`: `cargo fmt --check` clean,
`cargo clippy --all-targets -- -D warnings` clean, and all 28 test suites
pass. The branch introduces no Linux regression. The macOS behavior it
addresses was not reproducible from this side and is taken as reported.

## Findings that should be kept as they are

`temporary_directory()` in `src/app/tests/mod.rs`, applied across roughly
thirty fixtures, is the correct root-cause fix. macOS advertises `/var/...`
through `TMPDIR` while resolving `/private/var/...`, and application paths are
canonicalized when they become buffer and workspace identities, so fixtures
that began from the advertised spelling were comparing aliases. Canonicalizing
at the fixture root removes the mismatch at its source. The same change in
`src/config.rs`, `src/file_picker.rs`, `src/ui.rs`, and the integration suites
follows the same shape.

The `git_monitor.rs` change is the strongest in the branch. A `sync` followed
by a 75 ms sleep and three real filesystem writes became a `Barrier` message
round-tripped through the worker queue, followed by direct injection of the
native events. That removes both the elapsed-time guess and the dependence on
the platform notification backend, whose coalescing and latency differ between
Linux and Darwin. The `#[cfg(test)]` seam in the worker enum matches the
existing convention in `src/git/cli.rs`.

`durable_completion_wins_a_race_with_launcher_loss` is rebuilt around
`RUNYTE_TEST_WAIT_STATUS_BARRIER`, which parks the wait client after it has
sent a status request and before it can consume the reply. The ordering the
test needs is now constructed rather than hoped for, and the
`waiter.try_wait()` assertion it replaces was itself a statement about
scheduling. The environment-variable seam matches the precedent set by
`RUNYTE_TEST_SUPERVISOR_PID`.

The colour adaptation in `cedf0c2` is architecturally sound. Detection is
client-owned and performed once from Crossterm's conservative advertisement,
hosts retain exact RGB in their semantic snapshots, and every
`render_host_frame` call site in the attach paths is converted, so clients on
different terminals render one workspace at their own depth without altering
it. The arithmetic is correct: the cube levels, the `16 + 36r + 6g + b` index,
the `8 + 10i` grayscale ramp over 232..255, and the `xterm_color` decode all
match the xterm palette, including the case where an average below 8 correctly
selects the cube rather than the ramp. Excluding indices 0 through 15 as
quantization targets because a terminal profile may redefine them is right.
Both `context/reference/terminal-compatibility-v1.md` and `docs/user-guide.md`
were updated, as changing terminal behavior requires.

`boot_namespace_from_identifier` is a good extraction and adds coverage of the
empty and oversized rejections. The `external_open` process-group test is
better for measuring `getpgid` directly instead of parsing `ps` output.

## Findings that should be corrected before the branch lands

Five assertions in `tests/persistent_host.rs` were converted from failures
into silent passes:

```rust
-    assert!(wait_for_endpoint(&mut original, &endpoint).await, "host did not become ready");
+    if !wait_for_endpoint(&mut original, &endpoint).await { fs::remove_dir_all(root).unwrap(); return; }
```

A host that never becomes ready is the failure this suite exists to detect,
and the test now reports success having verified nothing. `start_host` in
`tests/local_protocol.rs` is not a precedent for this: it returns early only
after identifying one specific environmental refusal in the child's stderr,
where the sandbox denies Unix sockets. A readiness timeout carries no such
identification. If macOS has a specific refusal here it should be detected as
one; otherwise these must remain assertions.

Two assertions were deleted without replacement.
`assert!(commit.try_wait().unwrap().is_none())` established that the
`git commit` process is still parked while the editor holds `COMMIT_EDITMSG`;
the analogous race in the launcher-loss test was given a barrier, and this one
should have the same treatment or a protocol-level equivalent that the wait
request is still outstanding. `assert!(stderr.contains("raw mode"))` in
`tests/diagnostic_log.rs` was pinning a Crossterm-internal string, which is a
fair objection, but the replacement is a comment; the test should still assert
that the failure has a recognized frontend-initialization shape.

All three commits have empty bodies. This history explains diagnoses rather
than listing changes, and `AGENTS.md` requires a resolved record to state what
was wrong and why the fix is shaped as it is. That material does not yet exist
for any of this work.

## Findings worth addressing

`/tmp` is hardcoded in six places across `src/main.rs`, `src/workspace/catalog.rs`,
`tests/diagnostic_log.rs`, `tests/persistent_host.rs`, `tests/local_protocol.rs`,
and `tests/workspace_bulk.rs`, in four different spellings, bypassing `TMPDIR`.
Darwin's Unix-domain socket path limit is a real constraint, but this is one
policy duplicated six times and it should be a single helper. It is also worth
confirming the constraint binds at all: `test_runtime_dir()` already keeps its
base name short enough that the advertised macOS temporary directory fits
under the limit once the endpoint path is appended.

`unique_test_root` now ignores its `label` parameter, so catalog fixture
directories are no longer identifiable by test. Either keep a short label or
remove the parameter.

No test calls `boot_namespace()` any more, so a regression in
`boot_identifier()` would go uncaught on every platform. The real function
should still be asserted on where the sysctl succeeds, with the skip narrowed
to the sandbox failure that motivated the change.

`symlinked_owner_wide_inventory_is_refused_without_following_it` was narrowed
from `LocalServer::bind` to `publish_metadata`. That refusal is a security
property, and it now has no end-to-end coverage: if `bind` stopped calling
`publish_metadata`, nothing would notice.

`tests/directory_buffer.rs` switched its fixture from Enter to `P`, so the
default trash path has no coverage anywhere. The motivation is understandable,
since the platform trash needs Finder access that CI does not have, but the
gap should be recorded rather than left implicit.

`git init -q --initial-branch=master` correctly removes the dependence on the
machine's `init.defaultBranch`. Two questions remain: why `master` when this
repository's own default branch is `main`, and whether pinning the flag sets a
Git 2.28 floor worth stating.

`context/reference/terminal-compatibility-v1.md` states that explicitly named
ANSI theme colours retain their semantic terminal names, but the `Basic` depth
remaps `White` to `Gray` and `DarkGray` to `Black`. The code is right, because
those two are bright variants that an eight-colour terminal cannot show; the
reference needs the exception.

`with_extension("ready")` and `with_extension("release")` replace an extension
rather than appending one, so both barriers break if their base path ever
gains a suffix. `pub fn render` now silently defaults to `TrueColor`, which
suits the three test backends that call it but would let a future production
caller bypass adaptation without any signal.

## Merge considerations

`dev` has moved on by five commits since `b09c444`. Three files are touched by
both lines of work.

`src/external_open.rs` conflicts. Both branches rewrote
`launched_program_has_a_process_group_separate_from_the_editor`: this branch
replaced `ps` output parsing with a direct `getpgid` call and a release-file
hold, while `2aafb9d` on `dev` replaced the runtime-written script with the
checked-in `src/fixtures/stand-in` to remove an `ETXTBSY` failure. Both
changes are wanted, and the resolution is the `getpgid` logic with the
behavior installed through `install_stand_in`.

`tests/local_protocol.rs` merges without conflict, but this branch's `/tmp`
change lands in `test_runtime_dir()`, which is also where `dev` places the
process-audit journal described in
`context/reviews/git_child_sigkill_on_darwin.md`. A clean automatic merge is
not evidence that the result is correct there.

`tests/git_provider.rs` merges without conflict.

None of this branch addresses the Darwin `SIGKILL` failure recorded in
`context/reviews/git_child_sigkill_on_darwin.md`; the two lines of work are
independent.
