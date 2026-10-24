# Releasing Runyte to crates.io

This is the register of record for cutting a release. Follow it as written.
Do not infer the version scheme, the branch, or the publish flags from the
commit history: several of the choices below are deliberate and are not
visible in the diffs.

## What a release is

A release after the initial public snapshot has one version-only commit on `main`, subject
`Release <version>`, touching exactly two files: `Cargo.toml` and `Cargo.lock`.
Nothing else belongs in it. The code being released is already merged before
the version is bumped. `main` is protected: both code integration and the
version bump land through pull requests (merge requests), never a direct push.
Squash the single-commit version PR with the subject `Release <version>` so
the resulting commit on `main` still changes exactly those two files.

Each release also has a GitHub Release for its version tag. Its body is a
curated `## Changes` list describing what changed since the previous version,
not a dump of commit subjects. Release notes are drafted outside the working
tree, so they do not become a third file in the release commit.

The `Binary release` GitHub Actions workflow attaches five terminal archives,
two Linux desktop archives, and one combined `SHA256SUMS` to that GitHub Release. It builds the exact tag on
native x86-64 and ARM64 Linux/macOS runners and an x86-64 Windows MSVC runner. Cargo publishing remains a
manual local operation; the workflow never publishes to crates.io.

The public Git history was reset after version 0.0.51. Earlier versions remain
available from crates.io, but their private-development commits and release
tags are not reproduced in the public repository. The cleaned root snapshot is
version 0.1.0 and is the one exception to the separate two-file release-commit
rule above. If published, that root commit receives the `v0.1.0` tag.

Releases up to 0.0.9 used the subject `Release runyte <version>`. The crate
name was dropped at 0.0.10; do not reintroduce it.

## Versioning

Runyte is pre-1.0. The current release line is 0.4.x and routine releases
bump the patch component: 0.4.0 becomes 0.4.1. A later minor-version change
requires an explicit compatibility or scope decision rather than happening as
part of an ordinary release.

Stable plugin support begins at **0.3.0**, with the clean `runyte-1` cutover.
Within a pre-1.0 `0.X` line, patch releases preserve the public plugin contract.
A minor boundary may break it only after an explicit decision. After 1.0,
minor/patch releases preserve compatibility and major releases may break it.
See [the compatibility register](plugin-compatibility.md) for the behavioral
promise, release-range rules, and frozen acceptance inventory. This strengthens
the patch-release policy prospectively; it does not cover experimental releases.

The version is written in one place, `[package] version` in `Cargo.toml`.
`Cargo.lock` holds a copy that Cargo rewrites for you; never hand-edit it.

## The runbook

The example version below is 0.4.1. Substitute the real one.

1. **Merge all intended work through a PR from `dev` to `main`.** Fetch
   current branches, review the complete difference, and open or reuse the
   integration PR. Require successful `CI` and `Desktop edition` workflows
   before merging. Use a merge commit to preserve the shared development
   history. Do not bypass protection or push directly to `main`.

   If `dev` holds work that is not meant to ship yet, stop and settle that
   before continuing rather than releasing a subset. After the PR is merged:

   ```sh
   git fetch origin
   git switch main
   git pull --ff-only origin main
   git switch -c release/0.4.1
   ```

   The release branch starts at the merged `main`. Keep unrelated work out of
   `main` until publication completes; if it advances, re-evaluate the candidate
   and repeat the affected gates and notes review.

2. **Confirm the tree is clean.** `git status` must report nothing. Cargo
   refuses to publish from a dirty working directory, and an unrelated stray
   change would otherwise end up inside the release commit.

3. **Write the release's `Changes` notes.** Compare the release candidate with
   the previous version tag. For 0.4.1 that range is `v0.4.0..HEAD`:

   ```sh
   git log --no-merges --reverse --format='%h %s' v0.4.0..HEAD
   git diff --stat v0.4.0..HEAD
   ```

   Inspect the commits and their diffs rather than deriving the notes from
   subjects alone. Write the result to
   `/tmp/runyte-v0.4.1-CHANGES.md`, beginning with `## Changes`. It must cover
   every material user-facing feature, behavior change, fix, and performance
   improvement, call out any compatibility or migration concern, and omit
   merges, the version-only release commit, and internal work that has no
   effect on users. Group related work into concise prose or bullets and end
   with a link to the full comparison:

   ```markdown
   [Full comparison](https://github.com/runyte/runyte/compare/v0.4.0...v0.4.1)
   ```

   Read the finished notes once against the range to catch omissions and claims
   the diff does not support. Keep this file until step 13. It lives outside
   the repository so `git status` remains clean. Confirm that the GitHub CLI is
   ready to publish it before proceeding:

   ```sh
   gh auth status
   ```

4. **Run the gates** on the release branch:

   ```sh
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   cargo clippy -p runyte-native -p runyte-desktop -p runyte-preview --all-targets -- -D warnings
   cargo test -p runyte-native -p runyte-desktop -p runyte-preview
   ```

   Preserve the terminal and desktop coverage floors and performance budgets.
   Include real-TTY acceptance for terminal presentation changes and the
   applicable native window/preview acceptance for desktop changes. CI supplies
   the cross-platform evidence; a local pass does not replace it.

5. **Bump the version**, then refresh the lock file:

   ```sh
   cargo check
   ```

   Editing `Cargo.toml` alone leaves `Cargo.lock` naming the old version. Any
   Cargo command rewrites it; `cargo check` is the cheapest.

6. **Commit the bump.** Only the two files, and no other change:

   ```sh
   git add Cargo.toml Cargo.lock
   git commit -m "Release 0.4.1"
   ```

   This comes before the dry run because `cargo publish` reads the committed
   tree and rejects uncommitted changes.

7. **Dry-run the publish:**

   ```sh
   cargo publish -p runyte --locked --dry-run
   ```

8. **Land the version-only PR before publishing.** Push the release branch:

   ```sh
   git push -u origin release/0.4.1
   ```

   Open a PR targeting `main`, titled `Release 0.4.1`. Its diff must contain
   only `Cargo.toml` and `Cargo.lock`, with no dependency drift. Require green
   `CI` and `Desktop edition` PR checks, then squash-merge with commit subject
   `Release 0.4.1`. Do not bypass protection. The squash creates a new SHA;
   the branch's dry run and PR checks do not certify that final commit.

   ```sh
   git switch main
   git pull --ff-only origin main
   git status --short
   git log -1 --format='%H %s'
   git diff-tree --no-commit-id --name-only -r HEAD
   cargo publish -p runyte --locked --dry-run
   ```

   Confirm this is the PR's resulting commit, with the expected subject and
   exactly the two version files, and that the tree is clean. Stop if `main`
   has advanced beyond it; do not publish an unrelated later commit.

9. **Require green CI for the exact release commit.** Wait for both `CI`
   and `Desktop edition` push workflows triggered by merging step 8 and verify
   their head SHA is exactly `git rev-parse HEAD`. Every required job,
   including both lifecycle-stress platforms, plugin conformance, the immutable
   plugin inventory, and every frozen-plugin cell on Linux/macOS, must complete
   successfully. The separately marked moving-head external job is advisory
   and cannot replace immutable acceptance. Branch protection may enforce fewer
   checks than this runbook: PR eligibility alone is not release acceptance.

   The compatibility jobs must report `mode: exact` and exercise the version in
   this release commit. A staged minor-version candidate is development
   evidence only. No required native test may be skipped, and every referenced
   repository SHA must be fetchable independently. Publish the reviewed source
   pins before cutting this release; never rewrite a frozen client to repair a
   within-range host regression. A green run for an earlier commit
   does not qualify, and a failed job must be diagnosed and fixed rather than
   rerun until it happens to pass. Do not publish or tag while this check is
   pending or failed.

10. **Publish:**

   ```sh
   cargo publish -p runyte --locked
   ```

   This needs a crates.io token from `cargo login`, held in the local Cargo
   configuration. It is not in the repository and should never be searched
   for; if the token is missing, stop and say so.

11. **Tag the release commit and push the tag:**

   ```sh
   git tag v0.4.1
   git push origin v0.4.1
   ```

    Pushing the tag starts the `Binary release` workflow. It validates the tag
    and package version, builds and smoke-tests all five native targets, then
    creates or updates the GitHub Release and its binary assets. It does not
    move or create the tag.

12. **Require a green binary workflow for the exact tag.** Wait for the
    `Binary release` workflow triggered by step 11. Verify that its event is
    the pushed tag and that the validated source SHA is the release commit.
    Terminal/Linux desktop builds and the publishing job must pass. The
    independent universal macOS desktop job does not block those assets; its
    failure blocks manual macOS distribution. The expected workflow assets are
    seven archives plus `SHA256SUMS` (eight files); the macOS app zip is only a
    workflow artifact. Diagnose a failed build;
    rerunning a failed job is safe because publishing replaces only assets
    with the expected names and preserves the release body.

    After step 12, the maintainer may complete the manual signing, notarization,
    quarantine and real-window acceptance in the editions plan's Phase 8.
    Upload `Runyte-<version>.dmg` and its separate `.dmg.sha256` only after that
    acceptance. These two manual assets are excluded from workflow `SHA256SUMS`,
    which reruns regenerate. With the manual pair, the release has ten files;
    otherwise it has the eight workflow files. No signing identity or notary
    credential is stored in the repository.

13. **Publish the `Changes` notes.** The workflow creates the GitHub Release
    with a minimal binary-download note when it does not already exist. Replace
    that note with the curated draft without changing the tag or assets:

    ```sh
    gh release edit v0.4.1 \
      --title "Runyte 0.4.1" \
      --notes-file /tmp/runyte-v0.4.1-CHANGES.md
    ```

    Open the URL printed by `gh` and verify the title, tag, comparison link,
    rendered `Changes` list, seven archives, and `SHA256SUMS`, plus the manual
    DMG/checksum pair if macOS acceptance is complete. The GitHub Release
    is the published changes record; the temporary draft may then be removed.

14. **Carry the release commit back to `dev`**, so the branches do not diverge
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

Use a separate worktree for `release/0.4.1`, based on the updated `main`
worktree after the integration PR. Run steps 2 through 7 there. After the
version PR merges, run step 8's final checks and steps 9 through 13 in the
`main` worktree. Run step 14's merge and push in the `dev` worktree, omitting
its `git switch` lines. `git worktree list` identifies the existing trees.

Keep each tree clean. The final dry run and exact-commit CI must refer to the
merged version commit on `main`, not the pre-squash release branch or `dev`.

## Why these choices

**`--locked` on publish.** `README.md` tells installers to use
`cargo install runyte --locked`, so the crate is meant to be built against the
dependency graph in `Cargo.lock`. Passing `--locked` at publish time keeps the
publish honest about that: it ships the same resolved graph the gates in step 4
ran against, and fails loudly if the lock file is stale rather than quietly
resolving something newer.

**PR merge and exact-commit CI before publish.** A crates.io version cannot be
replaced or unpublished, only yanked. From 0.1.0 onward, merging through a PR first means
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

## Binary archives

The tag workflow publishes these files, where `<version>` includes its leading
`v`:

- `runyte-terminal-<version>-x86_64-unknown-linux-gnu.tar.xz`;
- `runyte-terminal-<version>-aarch64-unknown-linux-gnu.tar.xz`;
- `runyte-terminal-<version>-x86_64-apple-darwin.tar.xz`;
- `runyte-terminal-<version>-aarch64-apple-darwin.tar.xz`;
- `runyte-terminal-<version>-x86_64-pc-windows-msvc.zip`;
- `runyte-desktop-<version>-x86_64-unknown-linux-gnu.tar.xz`;
- `runyte-desktop-<version>-aarch64-unknown-linux-gnu.tar.xz`; and
- `SHA256SUMS`, covering all seven archives.

Each terminal archive has one top-level directory named
`runyte-terminal-MAJOR.MINOR.PATCH-<target>`. It contains the executable, `README.md`, `INSTALLATION.md`,
`LICENSE`, `NOTICE`, `THIRD_PARTY_NOTICES.md`, the complete `licenses/`
directory, and `config.example.yaml`. Linux archives are built on Ubuntu 22.04
to retain the glibc 2.35 floor. The macOS executables are unsigned and are not
notarized. Windows executables are also unsigned. The packaged README links to the current Windows support and limitations guide.

To verify one downloaded archive, compute `sha256sum <archive>` on Linux or
`shasum -a 256 <archive>` on macOS and compare the complete digest with the
archive's line in `SHA256SUMS`. On Windows use `Get-FileHash -Algorithm SHA256 <archive.zip>`. Downloading all seven archives permits the
direct `sha256sum -c SHA256SUMS` or `shasum -a 256 -c SHA256SUMS` form.

Build jobs keep only read access to repository contents. The final publishing
job alone receives `contents: write`, through the workflow-provided
`GITHUB_TOKEN`. Actions are pinned to complete commit hashes. A rerun checks
out the immutable commit resolved from the requested tag, replaces only the
eight expected assets, and neither recreates nor moves the tag. Build-provenance
attestations are deliberately omitted for now because they require additional
permissions; archive checksums and exact-tag validation add no such authority.

### Desktop release archives

The separate desktop job builds the native editor and the locked
`runyte-preview` workspace library from the same validated tag. It runs on Ubuntu
24.04 x86-64 and ARM64, so the desktop archives have a glibc 2.39 floor; terminal artifacts
retain their existing floor. Other desktop targets are not published until
native window and preview acceptance has been established on them.

`runyte-desktop-MAJOR.MINOR.PATCH-<target>/` contains `runyte`
with its internal helpers, `runed`, notices, icons, documentation
and the Linux desktop registration script.
The job extracts the completed archive and runs headless engine acceptance plus
real `:preview` window acceptance on isolated Xvfb/lavapipe, without a helper
environment override, before uploading. Publishing requires this job to succeed
and includes both desktop archives in `SHA256SUMS`.

Historical tags without `crates/runyte-desktop/Cargo.toml` retain their original
artifact set and `runyte-<tag>-<target>` names, and skip both desktop jobs.
The `macos-desktop` job builds the desktop executable and native launcher for ARM64 and x86-64 with `MACOSX_DEPLOYMENT_TARGET=11.0`, combines
both with `lipo`, and checks both slices for the deployment floor and system-only
library links. It runs headless engine acceptance, then retains
`Runyte-<version>-unsigned.app.zip` made by `ditto` as a workflow artifact only.
Publication requires this job to succeed for tags with desktop support; only
historical tags without desktop support may skip it. The publishing job does
not download its unsigned artifact.
The job also builds an unsigned disk image, attaches it read-only, checks its
contents and detaches it. That image is not published. The macOS window still
requires manual platform validation before distribution.

`contrib/packaging/sign_macos.py` signs the app inside-out with hardened runtime,
notarizes using a named existing Keychain profile, requires an Accepted status,
and staples. Its `dmg` command signs/notarizes/staples the image without runtime
options; `verify` checks both signatures and tickets through codesign, spctl and
stapler. `--dry-run` prints commands without invoking Apple tools. See the
[packaging guide](../../contrib/packaging/README.md) for command forms. This
workflow does not provision certificates, credentials, or entitlements.

### Curl installation and updates

The repository-root `install.sh`, served from `main`, installs and updates the
same executable. Its default resolves GitHub's `/releases/latest` redirect to
one stable version, then fetches that version's `SHA256SUMS` first. It selects
the listed `runyte-terminal-` archive, falling back to a listed historical
`runyte-` archive, and fails if neither is listed. `--version X.Y.Z` bypasses latest-release discovery.
`--install-dir` chooses an absolute destination directory; the default is
`$HOME/.local/bin`. Downloads and checks complete before a staged executable
replaces the old file through a rename on the destination filesystem. An
installed executable reporting the desktop edition is preserved: use the
desktop release archive to update it. Historical and unrecognized version
strings continue to count as terminal installations.

Installer changes land with ordinary code before the version-only release
commit. No installer version or embedded archive hash needs updating at release
time: each invocation verifies against the chosen release's manifest. The
existing crates.io, tag, and binary workflow order remains unchanged. A release
whose binary assets are not yet available fails installation clearly and can
be retried after its binary workflow completes.

The `installer` CI job runs `python3 -m unittest discover -s tests/installer -v`
on Linux and macOS. It exercises the shell script using real compressed
archives, checksum programs and filesystem replacements, with offline network
and platform fixtures. Changes to archive names, layout, compression or Linux
runtime requirements must keep the installer and its tests aligned.

### One-time 0.1.7 backfill

The workflow entered the repository after `v0.1.7`, so pushing that existing
tag again is neither necessary nor permitted. After `release.yml` is present
on `main`, dispatch `Binary release` from the Actions page with tag `v0.1.7`,
or run:

```sh
gh workflow run release.yml --ref main -f tag=v0.1.7
```

Wait for that manual run and verify its requested tag and validated source SHA
before accepting it. It updates the existing `Runyte 0.1.7` release without
replacing its curated notes. The same manual path can recover a failed future
tag workflow. Tags `v0.1.0` through `v0.1.6` exist, but retroactively creating
GitHub Releases for those crate versions is outside the current release
procedure.

## Notes

- The crate ships one binary, `runyte`.
- `context/`, `AGENTS.md`, and `tests/` are excluded from the published crate
  on purpose. `docs/` is included because `THIRD_PARTY_NOTICES.md` cites it.
- A release that changes terminal presentation should include an appropriate
  real-TTY acceptance pass in addition to the automated gates.
