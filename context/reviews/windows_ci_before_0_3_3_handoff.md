# Windows CI failures before the 0.3.3 release

Recorded: 2026-09-26. This is a handoff for work on a native Windows machine.
Remove it once Windows CI is green on `dev` and 0.3.3 is released; anything
durable belongs in `issues/resolved/` or a `reference/` register instead.

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
Windows run on `dev` is at `64ac2ca`. Every Linux and macOS job passed at
`addb538`. The failures fell into three independent groups. Each now has a
fix on `dev` that passes `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings` and `cargo test` on Linux, but **none has been run on Windows**.

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
does not compile on Linux, so the change has not been built at all.

## Steps that never ran

The `Native Windows` job stops after `Test native editing, Git and ConPTY`
fails. In the failing runs the steps `Provision the required native language
server` and `Accept native language-server approval and lifecycle` were
skipped, so those acceptances are unverified for everything since `64ac2ca`.

## To do on Windows

From the `dev` worktree at `0c2fcf6` or later:

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --no-fail-fast
```

then the ignored acceptances the workflow runs explicitly, as listed in the
`Native Windows` job of `.github/workflows/ci.yml`, including the language
server provisioning and acceptance steps. Fix whatever fails, push to `dev`,
and confirm a green `Native Windows` job on GitHub for the exact head commit
before returning to the release decision above.
