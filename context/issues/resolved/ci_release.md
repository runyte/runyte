---
title: "GitHub Releases lack automated native binary archives"
status: resolved
reported: 2026-08-26
resolved: 2026-09-01
commit: ee22ef9
---

## Resolution

Commit `ee22ef9` (`Automate binary releases`) added the missing release
automation. There was no defective editor function: the repository had a
manual crates.io runbook and an ordinary CI release-build floor, but no
tag-bound path that assembled or published native executables.

`.github/workflows/release.yml` now validates a pushed or manually supplied
semantic-version tag, resolves it to an immutable commit, and refuses a
`Cargo.toml` version mismatch before building. Four native jobs build and
smoke-test the Linux and macOS target matrix, package the executable with the
complete required documentation and licence material, and pass the archives
to a final Ubuntu job. That job alone receives `contents: write`; it verifies
the tag again, produces the combined `SHA256SUMS`, and creates or updates the
release with rerunnable asset uploads. Concurrency is keyed by the requested
tag so duplicate work contends without one version displacing a pending run
for another. All action dependencies are pinned to complete commit hashes, and
the manual tag reaches shell only through a quoted environment value.

`README.md` and `docs/user-guide.md` describe archive installation and checksum
verification. `context/reference/releasing.md` keeps crates.io publishing
manual, places the automated binary workflow after the tag push, explains the
four artifact names and unsigned macOS status, and records the manual `v0.1.7`
backfill. That is a deliberate boundary: only `v0.1.7` had an existing GitHub
Release when the workflow was added, while the generic dispatch remains able
to build an older valid tag if a later decision calls for it. Provenance
attestations were omitted because they would require additional permissions.

Regression coverage is in
`tests/release_packaging.rs::binary_release_is_tag_bound_native_and_narrowly_privileged`,
which parses the workflow and fixes its permissions, per-tag concurrency,
target/runner matrix, locked build, package manifest, checksum publication,
rerun behavior, and full-SHA action pins. The complete repository gates are
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo test`. The workflow YAML and all six shell blocks were also parsed
locally, and a real x86-64 Linux release archive probe verified its top-level
directory, required files, smoke test, and executable mode.

Known limitation: macOS archives are unsigned and not notarized. The Ubuntu
22.04 runner labels also have a scheduled deprecation, so preserving or moving
the glibc 2.35 binary floor will require a later explicit compatibility
decision.

## Report

Runyte has a crates.io release procedure but no automated GitHub binary
release. Linux and macOS users should be able to download archives built from
the exact version tag associated with a release. Windows is not currently a
supported target.

Versions 0.1.0 through 0.1.7 are already published on crates.io and their
matching tags, `v0.1.0` through `v0.1.7`, exist in the public repository. Only
`v0.1.7` currently has a GitHub Release. The binary-release workflow will
therefore be introduced after the first GitHub Release it must support. Its
documented one-time backfill is `v0.1.7`; backfilling the older crate tags is
not required by this issue, although the generic manual path may be used for
them later.

### Expected behavior

Future semantic-version tags matching `vMAJOR.MINOR.PATCH` trigger the binary
workflow automatically. A manual workflow dispatch with a required tag input
supports the one-time retroactive build of `v0.1.7` and any later recovery run.
Both paths check out and build the exact requested tag rather than the default
branch. The workflow validates the tag format, confirms that the tag exists,
and verifies that it matches `[package] version` in `Cargo.toml`; missing tags,
ambiguous input, and version mismatches are refused.

Cargo publishing and GitHub binary releases remain separate operations. The
documented manual procedure publishes successfully to crates.io before pushing
the matching tag. Pushing that tag starts the GitHub binary release.

The target matrix and current official GitHub-hosted runner labels, confirmed
on 2026-09-01, are:

- `x86_64-unknown-linux-gnu` on `ubuntu-22.04`;
- `aarch64-unknown-linux-gnu` on `ubuntu-22.04-arm`;
- `x86_64-apple-darwin` on `macos-15-intel`; and
- `aarch64-apple-darwin` on `macos-15`.

The Ubuntu 22.04 labels retain the glibc 2.35 compatibility floor already
checked by CI. GitHub has announced their deprecation beginning on 2026-09-17,
so moving the release floor will require an explicit compatibility decision
before those images become unavailable. `macos-15-intel` is GitHub's supported
Intel replacement label through August 2027.

Each target is built with
`cargo build --release --locked --target <target>` and smoke-tested with a safe
command such as `runyte --version` or `runyte --help`. Tree-sitter grammars are
statically linked, so no runtime grammar directory is packaged.

Each target produces a clearly named archive such as
`runyte-v0.1.7-x86_64-unknown-linux-gnu.tar.xz`. The archive has a versioned
top-level directory, preserves executable permissions, and contains:

- the `runyte` executable;
- `README.md`;
- `LICENSE`;
- `NOTICE`;
- `THIRD_PARTY_NOTICES.md`;
- the complete `licenses/` directory; and
- `config.example.yaml`.

Build jobs upload these archives as intermediate Actions artifacts. A final
Ubuntu publishing job downloads all four archives, generates one combined
`SHA256SUMS`, creates or updates the GitHub Release for the exact tag, uploads
the archives and checksum file, and uses a clear title such as
`Runyte 0.1.7`. A failed publishing job can be rerun safely and reasonably
idempotently without moving or recreating the tag.

### Security and maintenance constraints

- Build jobs have read-only repository permissions. Only the final publishing
  job receives `contents: write`.
- The workflow-provided `GITHUB_TOKEN` is sufficient; no personal token is
  required.
- The manual tag input is untrusted input and is validated and quoted before
  shell use.
- Third-party actions are pinned to full commit SHAs with comments naming
  their release versions.
- Official GitHub actions and the installed `gh` CLI are preferred over an
  unnecessary release-upload action.
- GitHub build-provenance attestations are considered only if they can be added
  cleanly with minimal permissions.
- Mutable release inputs and accidental builds from the wrong commit are not
  accepted.
- Windows artifacts are out of scope.
- Signing and macOS notarization are out of scope for this work. Documentation
  identifies the macOS binaries as unsigned.

Workflow development must not push, create tags, publish crates, create GitHub
Releases, or otherwise modify remote state before review.

### Documentation

`README.md` installation instructions should mention downloadable GitHub
Release archives while retaining `cargo install runyte --locked`.
`context/reference/releasing.md` should place the automated binary workflow
after a successful crates.io publish and tag push and should distinguish every
manual step from every GitHub-managed step.

The documentation should also record:

- how to run the manual `v0.1.7` backfill after the workflow reaches `main`;
- artifact names and supported targets;
- checksum verification;
- rerun behavior; and
- the fact that macOS artifacts are currently unsigned.

### Validation

The implementation should be checked against `AGENTS.md`, `README.md`,
`context/README.md`, `context/reference/releasing.md`, and the existing GitHub
workflows. Workflow YAML syntax must be validated. `actionlint` is used if it
is already available; otherwise the alternative validation is recorded without
installing unrelated global tools.

Repository tests covering packaging or release metadata must pass, followed
by:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The final review checks for excessive workflow permissions, unpinned actions,
unsafe interpolation, incorrect target selection, documentation
inconsistencies, and unrelated changes. The handoff records the files changed,
exact triggers and target matrix, the `v0.1.7` backfill procedure, security
decisions, validation results, and any remaining limitation, especially the
unsigned macOS binaries.
