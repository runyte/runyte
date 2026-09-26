# Windows CI failures before the 0.3.3 release

Recorded: 2026-09-26. This is a handoff for work on a native Windows machine.
Remove it once Windows CI is green on `dev` and 0.3.3 is released; anything
durable belongs in `issues/resolved/` or a `reference/` register instead.

## Windows verification

The Windows CI blocker is cleared at `39c8d8c` on `dev`.
[CI run 36255051583, Native Windows](https://github.com/runyte/runyte/actions/runs/36255051583/job/108440216289)
passed on 2026-09-26 with all three original fixes below. A separate native
run then exposed the intermittent fixture failures described in sections 4
and 5. The passing CI job includes:

- formatting, warnings-as-errors all-target Clippy, and the full ordinary
  native editing, Git and ConPTY suite;
- successful persistent-host replacement, refusal without breakaway policy,
  and complete save contents after acknowledgement;
- the Python context adapter checks, the real native-host MCP bridge, and
  the public Windows context round trip;
- all three isolated native clipboard acceptances; and
- rust-analyzer provisioning and the real permission, diagnostics, edits,
  restart and cleanup acceptance.

This verifies the previously skipped language-server steps as well as the
original failures. It does not publish 0.3.3 or settle the `main` history
decision below. The handoff remains until the release is complete.

Local verification uses Rust and rust-analyzer 1.97.1 on
`x86_64-pc-windows-msvc`, with `CARGO_BUILD_JOBS=1` and
`RUST_TEST_THREADS=2` as in CI. Formatting, all-target Clippy, the ordinary
suite, restart/save acceptances, both Rust context-bridge acceptances, and the
Python checks pass. The fixed language-server and session-switch acceptances
each passed ten consecutive runs. Python 3.13.7 was provisioned in temporary storage for the
adapter checks. The three local clipboard acceptances were refused when
creating their private window stations (`Access is denied`, OS error 5):
the local process lacks administrator privileges. Their successful evidence
is the required CI step above; the fixtures were not weakened or redirected
to a shared clipboard.

## Release state

Version 0.3.3 is **not** published. `cargo publish` ran only with `--dry-run`,
no `v0.3.3` tag exists, and no GitHub Release or binary workflow was started.
crates.io still ends at 0.3.2, so the version number remains free.

`origin/main` holds `addb538` (`Release 0.3.3`), a two-file version bump pushed
on top of `1d031bf`. Its `CI` run (36241362404) failed only in the
`Native Windows` job, so under `context/reference/releasing.md` step 9 it
cannot be published or tagged. How that commit is handled is undecided. The
options considered are:

1. reset `main` to `1d031bf` with `--force-with-lease`, fast-forward it to the
   fixed `dev`, and recreate `Release 0.3.3` as the only two-file commit on top;
2. revert `addb538`, merge `dev`, and add a new `Release 0.3.3` commit, leaving
   history append-only;
3. merge `dev` on top of `addb538` and tag the merge, which departs from the
   runbook's two-file tagged commit.

Do not choose among these without the maintainer; the first rewrites a
published branch.

The release notes draft for 0.3.3 lived outside the repository and is not
carried by this record. Regenerate it from `v0.3.2..HEAD` per the runbook,
including the three test fixes below only if they turn out to affect users
(they do not as written).

## What failed on Windows

`Native Windows` was already red on `dev` from `42a6f36` onward; the last green
Windows run before these fixes was at `64ac2ca`. Every Linux and macOS job
passed at `addb538`. The failures fell into three independent groups. Each had a
fix on `dev` that passed `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings` and `cargo test` on Linux when this handoff was written. Their native
Windows verification is now recorded above.

### 1. Lint: a Unix-only test helper was dead code

`cargo clippy --all-targets --locked -- -D warnings` failed with
`function 'destinations' is never used` at
`src/app/tests/session_navigation.rs`. Every caller of that helper is
`#[cfg(unix)]`, so on Windows it is dead code and `-D warnings` rejects it.
Introduced by `47e846a` (`Unify Navigator, buffer and terminal destination
lists`).

Fix: `6b2d6dc` (`Gate the Unix-only destination list test helper`) gives the
helper the same `#[cfg(unix)]` as its callers. CI run 36242016279 at that
commit confirmed the Lint step passes on Windows.

### 2. Committed comparison tests start `git` by bare name

Fourteen tests failed, eight in the library and six in `tests/git_provider.rs`:

```text
app::tests::git_comparison::committed_comparison_async_results_preserve_foreground_ownership
app::tests::git_comparison::committed_comparison_count_colors_follow_branch_and_worktree_refresh
app::tests::git_comparison::committed_comparison_keys_restore_file_and_scroll_from_patch_and_both_split_sides
app::tests::git_comparison::committed_comparison_refresh_resize_and_status_action_keep_semantic_targets
app::tests::git_comparison::committed_comparison_reprojects_visible_and_hidden_positions_across_resize_and_refresh
app::tests::git_comparison::committed_comparison_return_positions_belong_to_each_originating_pane
app::tests::git_comparison::committed_comparison_reuses_inactive_split_buffers
app::tests::git_comparison::committed_comparison_worktree_action_uses_the_selected_checkout_tip
committed_comparison_paths_are_literal_and_metadata_is_retained
committed_comparison_rename_patch_excludes_descendants_of_the_old_file_path
committed_comparison_service_reads_both_formats_and_reports_bounded_failures
committed_comparison_submodule_patch_normalizes_configured_log_format
committed_comparisons_capture_tips_paths_counts_and_immutable_file_versions
committed_comparisons_support_cached_remote_refs_and_detached_worktrees
```

The integration tests reported:

```text
Unavailable { detail: "cannot start `git`: background program must be an absolute native executable" }
```

and the library tests panicked at an `Option::unwrap()` in the `row` helper of
`src/app/tests/git_comparison.rs`, because the comparison buffer never
appeared for the same reason. The tests, added by `4f52a75` (`Add committed
branch and worktree comparisons`), built `GitCliProvider::new("git")`.
`src/windows_process.rs` refuses a background program that is not an absolute
path. The other Git tests resolve Git with
`GitCliProvider::discover(PATH)`, which returns an absolute path.

Fix: `dd30b29` (`Resolve Git to an absolute path in committed comparison
tests`) uses the existing `provider()` helper in `tests/git_provider.rs` and a
matching `git()` helper in `src/app/tests/git_comparison.rs`. Production code
already resolves Git by discovery; this is a test-only defect.

`src/app/tests/language.rs` also constructs `GitCliProvider::new("git")` in
`git_project_availability_distinguishes_missing_git_and_non_repository`. That
test checks command availability after installing the provider, passed on
Windows, and was left unchanged.

### 3. Windows manager-visit fixture waits for an old title

```text
native_manager_visits_selected_live_publication_and_refuses_stale_row
public_manager_visit_fixture
```

in `tests/windows_public_attachment.rs` (a `#![cfg(windows)]` file) timed out
with `Enter visit ·: Err(Timeout)`. The captured screen showed the session
manager open, titled `Sessions · Enter open · Tab actions · Esc close`.
`525facb` (`Name the session manager's keys in a legend and its Tab menu`)
renamed that title from `Enter visit`, and its Unix tests were updated, but
this Windows-only fixture still waited for `Enter visit ·`.

Fix: `0c2fcf6` (`Wait for the session manager's current title in the Windows
visit fixture`) replaces the three occurrences with `Enter open ·`. This file
does not compile on Linux; its build and execution are now verified by the
passing native Windows job above.

### 4. Native language-server formatting raced document processing

The local explicit
`real_rust_analyzer_permission_diagnostics_edits_restart_and_cleanup`
acceptance in `tests/lsp_windows_acceptance.rs` failed in its reexecuted
`native_lsp_fixture` with:

```text
formatting failed: Failed("formatting: content modified")
```

`manager` sent `LspCommand::Change` for version 2 and immediately requested
formatting. Queue order alone did not establish that rust-analyzer had
processed the changed document before starting the formatting request.
The fixture now waits for `LspEvent::Diagnostics` with version 2 and the
same canonical document path before requesting formatting. The existing
bounded event deadline still fails if the server never acknowledges that
version; no sleep or retry masks a formatting failure. The real-server
acceptance continues to verify returned edits, restart and revocation cleanup.
This changes only the Windows acceptance fixture, not editor behavior.

### 5. Session-switch cancellation read an unfinished fixture record

A subsequent native ordinary-suite run failed
`windows_frontend_acceptance::native_frontend_switches_between_exact_running_hosts`
in `src/tui/windows_frontend_acceptance.rs`. Its reexecuted
`switch_parent_fixture` observed `directory-cancel-process.json` and then
failed to deserialize it with `EOF while parsing a value`, line 1, column 0.

`parent_attach_host_fixture` used `fs::write` directly on that final path.
The parent waited for path existence, which could become true after file
creation but before the JSON bytes were written. The child now writes and
closes `directory-cancel-process.pending`, then renames it to the final name.
This follows the existing switch-target and directory-request fixture
publication pattern. The parent still fails on malformed published data;
there is no parse retry. The real session-switch acceptance covers cancellation
of the provisional host and retirement of its publication.

## Steps skipped in the failing runs

The `Native Windows` job stops after `Test native editing, Git and ConPTY`
fails. In the failing runs the steps `Provision the required native language
server` and `Accept native language-server approval and lifecycle` were
skipped. Both now pass at `39c8d8c` in the job linked above.

## Reproducing Windows verification

From the `dev` worktree at `0c2fcf6` or later:

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --no-fail-fast
```

Then run the ignored acceptances the workflow runs explicitly, as listed in the
`Native Windows` job of `.github/workflows/ci.yml`, including the language
server provisioning and acceptance steps. A restricted sandbox can refuse the
private named-pipe connections these tests require; use a normal native
process environment. The isolated clipboard fixtures additionally require
administrator privileges and must never fall back to the user's clipboard.

## Remaining release work

Confirm the required CI results for the final `dev` head, then obtain the
maintainer's decision about `main` above and follow
`context/reference/releasing.md` for the release candidate. A green Windows
job on `dev` does not make the failed `addb538` release candidate publishable.
