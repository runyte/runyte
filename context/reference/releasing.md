# Releasing Runyte to crates.io

This is the register of record for cutting a release. Follow it as written.
Do not infer the version scheme, the branch, or the publish flags from the
commit history: several of the choices below are deliberate and are not
visible in the diffs.

## What a release is

A release after the initial public snapshot is one commit on `main`, subject
`Release <version>`, touching exactly two files: `Cargo.toml` and `Cargo.lock`.
Nothing else belongs in it. The code being released is already merged before
the version is bumped.

Each release also has a GitHub Release for its version tag. Its body is a
curated `## Changes` list describing what changed since the previous version,
not a dump of commit subjects. Release notes are drafted outside the working
tree, so they do not become a third file in the release commit.

The public Git history was reset after version 0.0.51. Earlier versions remain
available from crates.io, but their private-development commits and release
tags are not reproduced in the public repository. The cleaned root snapshot is
version 0.1.0 and is the one exception to the separate two-file release-commit
rule above. If published, that root commit receives the `v0.1.0` tag.

Releases up to 0.0.9 used the subject `Release runyte <version>`. The crate
name was dropped at 0.0.10; do not reintroduce it.

## Versioning

Runyte is pre-1.0. The public release line begins at 0.1.0 and routine releases
bump the patch component: 0.1.0 becomes 0.1.1. A minor-version change requires
an explicit compatibility or scope decision rather than happening as part of
an ordinary release.

The version is written in one place, `[package] version` in `Cargo.toml`.
`Cargo.lock` holds a copy that Cargo rewrites for you; never hand-edit it.

## The runbook

The example version below is 0.1.1. Substitute the real one.

1. **Get onto `main` and take everything.** Releases are cut from `main`, and
   `main` must first contain the work being released.

   ```sh
   git switch main
   git pull
   git merge dev
   ```

   The merge fast-forwards whenever `main` is already an ancestor of `dev`,
   which is the usual case. If `dev` holds work that is not meant to ship yet,
   stop and settle that before continuing rather than releasing a subset.

2. **Confirm the tree is clean.** `git status` must report nothing. Cargo
   refuses to publish from a dirty working directory, and an unrelated stray
   change would otherwise end up inside the release commit.

3. **Write the release's `Changes` notes.** Compare the release candidate with
   the previous version tag. For 0.1.1 that range is `v0.1.0..HEAD`:

   ```sh
   git log --no-merges --reverse --format='%h %s' v0.1.0..HEAD
   git diff --stat v0.1.0..HEAD
   ```

   Inspect the commits and their diffs rather than deriving the notes from
   subjects alone. Write the result to
   `/tmp/runyte-v0.1.1-CHANGES.md`, beginning with `## Changes`. It must cover
   every material user-facing feature, behavior change, fix, and performance
   improvement, call out any compatibility or migration concern, and omit
   merges, the version-only release commit, and internal work that has no
   effect on users. Group related work into concise prose or bullets and end
   with a link to the full comparison:

   ```markdown
   [Full comparison](https://github.com/runyte/runyte/compare/v0.1.0...v0.1.1)
   ```

   Read the finished notes once against the range to catch omissions and claims
   the diff does not support. Keep this file until step 12. It lives outside
   the repository so `git status` remains clean. Confirm that the GitHub CLI is
   ready to publish it before proceeding:

   ```sh
   gh auth status
   ```

4. **Run the gates**, the same three as any handoff:

   ```sh
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

5. **Bump the version**, then refresh the lock file:

   ```sh
   cargo check
   ```

   Editing `Cargo.toml` alone leaves `Cargo.lock` naming the old version. Any
   Cargo command rewrites it; `cargo check` is the cheapest.

6. **Commit the bump.** Only the two files, and no other change:

   ```sh
   git add Cargo.toml Cargo.lock
   git commit -m "Release 0.1.1"
   ```

   This comes before the dry run because `cargo publish` reads the committed
   tree and rejects uncommitted changes.

7. **Dry-run the publish:**

   ```sh
   cargo publish --locked --dry-run
   ```

8. **Push `main` before publishing:**

   ```sh
   git push origin main
   ```

9. **Require green CI for the exact release commit.** Wait for the `CI`
   workflow triggered by step 8 and verify that its head SHA is exactly the
   output of `git rev-parse HEAD`. Every job, including both lifecycle-stress
   platforms, must complete successfully. A green run for an earlier commit
   does not qualify, and a failed job must be diagnosed and fixed rather than
   rerun until it happens to pass. Do not publish or tag while this check is
   pending or failed.

10. **Publish:**

   ```sh
   cargo publish --locked
   ```

   This needs a crates.io token from `cargo login`, held in the local Cargo
   configuration. It is not in the repository and should never be searched
   for; if the token is missing, stop and say so.

11. **Tag the release commit and push the tag:**

   ```sh
   git tag v0.1.1
   git push origin v0.1.1
   ```

12. **Publish the `Changes` notes.** Create a GitHub Release from the tag that
    was just pushed. `--verify-tag` prevents `gh` from silently creating a tag
    at some other commit:

    ```sh
    gh release create v0.1.1 \
      --verify-tag \
      --title "Runyte 0.1.1" \
      --notes-file /tmp/runyte-v0.1.1-CHANGES.md
    ```

    Open the URL printed by `gh` and verify the title, tag, comparison link,
    and rendered `Changes` list. The GitHub Release is the published changes
    record; the temporary draft may then be removed.

13. **Carry the release commit back to `dev`**, so the branches do not diverge
    over a version bump:

    ```sh
    git switch dev
    git merge main
    git push origin dev
    git switch main
    ```

## When `main` and `dev` are separate worktrees

The runbook above is written for one working tree that switches between the two
branches. Where each branch has a worktree of its own, `git switch` refuses:

```
fatal: 'main' is already used by worktree at <path>
```

That is the only thing that changes. Run steps 1 through 12 from whichever
worktree holds `main`, and step 13's merge from the one holding `dev`, dropping
the two `git switch` lines — a worktree keeps its branch, so there is nothing to
switch back to at the end. `git worktree list` names which is which. Every
command in between is unchanged, and so are the commits and pushes it produces.

Take the same care over step 2 in both trees. A release is cut from `main`, but
the gates in step 4 and the version bump in step 5 write build output and edit
`Cargo.toml` in the tree they are run from, so it has to be the same one
throughout. Running the gates in the `dev` tree checks something other than what
is about to ship.

## Why these choices

**`--locked` on publish.** `README.md` tells installers to use
`cargo install runyte --locked`, so the crate is meant to be built against the
dependency graph in `Cargo.lock`. Passing `--locked` at publish time keeps the
publish honest about that: it ships the same resolved graph the gates in step 4
ran against, and fails loudly if the lock file is stale rather than quietly
resolving something newer.

**Push and exact-commit CI before publish.** A crates.io version cannot be
replaced or unpublished, only yanked. From 0.1.0 onward, pushing first means
every published version has a matching public commit, even if the publish then
fails. Waiting for that commit's complete CI result also prevents an unrelated
earlier green run or a lucky rerun from authorizing a release. The reverse
order can leave a version on crates.io that corresponds to nothing anyone can
fetch.

**The dry run.** `Cargo.toml` sets `include` as an allowlist, so a file that is
not listed is silently absent from the published crate rather than flagged. The
dry run packages the crate for real and is the only cheap check that the
allowlist still covers everything the build needs. It matters most after a
change that adds a new non-source input — an asset, a licence file, a
configuration sample.

**Curated changes rather than generated notes.** Commit boundaries and subjects
describe development history; they are not reliably the units a user needs to
understand. Reading the whole previous-tag range makes omissions less likely,
while a curated list can combine related commits, exclude merge mechanics, and
state compatibility effects directly. Publishing that list as a GitHub Release
keeps it beside the corresponding public tag without changing the deliberately
two-file release commit or the crate contents.

**Tags.** The reconstructed public history does not reproduce tags through
`v0.0.51`. Do not backfill or retarget them. The root snapshot may receive
`v0.1.0`; tag each later release normally.

## Notes

- The crate ships one binary, `runyte`.
- `context/`, `AGENTS.md`, and `tests/` are excluded from the published crate
  on purpose. `docs/` is included because `THIRD_PARTY_NOTICES.md` cites it.
- A release that changes terminal presentation should include an appropriate
  real-TTY acceptance pass in addition to the automated gates.
