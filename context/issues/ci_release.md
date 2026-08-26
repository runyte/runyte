# Automated GitHub binary releases

Runyte has a crates.io release procedure but no automated GitHub binary
release. Linux and macOS users should be able to download archives built from
the exact version tag associated with a release. Windows is not currently a
supported target.

Version 0.1.0 is already published on crates.io, and tag `v0.1.0` points to the
initial public commit. The binary-release workflow will therefore be introduced
after the first tag it must support.

## Expected behavior

Future semantic-version tags matching `vMAJOR.MINOR.PATCH` trigger the binary
workflow automatically. A manual workflow dispatch with a required tag input
supports the one-time retroactive build of an existing tag such as `v0.1.0`.
Both paths check out and build the exact requested tag rather than the default
branch. The workflow validates the tag format, confirms that the tag exists,
and verifies that it matches `[package] version` in `Cargo.toml`; missing tags,
ambiguous input, and version mismatches are refused.

Cargo publishing and GitHub binary releases remain separate operations. The
documented manual procedure publishes successfully to crates.io before pushing
the matching tag. Pushing that tag starts the GitHub binary release.

The target matrix is:

- `x86_64-unknown-linux-gnu` on an Ubuntu 22.04 x86-64 runner;
- `aarch64-unknown-linux-gnu` on an Ubuntu 22.04 ARM64 runner;
- `x86_64-apple-darwin` on an Intel macOS runner; and
- `aarch64-apple-darwin` on an Apple Silicon macOS runner.

Current official GitHub runner labels must be confirmed when the workflow is
implemented. The Intel macOS label is the unstable one: `macos-13` was the last
Intel image under the classic naming, while `macos-15-intel` was introduced as
its replacement as GitHub retired Intel images.

Each target is built with `cargo build --release --locked` and smoke-tested
with a safe command such as `runyte --version` or `runyte --help`. Tree-sitter
grammars are statically linked, so no runtime grammar directory is packaged.

Each target produces a clearly named archive such as
`runyte-v0.1.0-x86_64-unknown-linux-gnu.tar.xz`. The archive has a versioned
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
`Runyte 0.1.0`. A failed publishing job can be rerun safely and reasonably
idempotently without moving or recreating the tag.

## Security and maintenance constraints

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

The implementation is prepared locally and left uncommitted for review. It
does not push, create tags, publish crates, create GitHub Releases, or otherwise
modify remote state while the workflow itself is being developed.

## Documentation

`README.md` installation instructions should mention downloadable GitHub
Release archives while retaining `cargo install runyte --locked`.
`context/reference/releasing.md` should place the automated binary workflow
after a successful crates.io publish and tag push and should distinguish every
manual step from every GitHub-managed step.

The documentation should also record:

- how to run the manual `v0.1.0` backfill after the workflow reaches `main`;
- artifact names and supported targets;
- checksum verification;
- rerun behavior; and
- the fact that macOS artifacts are currently unsigned.

## Validation

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
exact triggers and target matrix, the `v0.1.0` backfill procedure, security
decisions, validation results, and any remaining limitation, especially the
unsigned macOS binaries.
