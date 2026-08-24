Set up automated GitHub binary releases for Runyte.

Before changing anything, read:

- AGENTS.md
- README.md
- context/README.md
- context/reference/releasing.md

Inspect the repository and existing GitHub workflows before designing the solution. Preserve unrelated changes. Do not commit, push, create tags, publish crates, create GitHub Releases, or otherwise modify remote state. Leave the implementation uncommitted for review.

Context:

- Runyte is a Rust terminal editor supporting Linux and macOS; Windows is not currently supported.
- Version 0.1.0 is already published on crates.io.
- Tag v0.1.0 already exists and points to the initial public commit.
- The release workflow will be added after that tag, so it must support manually building an existing tag such as v0.1.0.
- Future version tags should trigger the workflow automatically.
- Cargo publishing and GitHub binary releases remain separate operations.
- Tree-sitter grammars are statically linked; no runtime grammar directory should be packaged.

Implement a secure GitHub Actions workflow that:

1. Runs automatically when a semantic version tag matching `vMAJOR.MINOR.PATCH` is pushed.
2. Supports `workflow_dispatch` with a required tag input so v0.1.0 can be built retroactively.
3. Always checks out and builds the exact requested tag, never merely the default branch.
4. Validates the tag format and confirms that it matches `[package] version` in Cargo.toml.
5. Refuses missing tags, version mismatches, or ambiguous input.
6. Builds with `cargo build --release --locked` for:

   - `x86_64-unknown-linux-gnu` on an Ubuntu 22.04 x86-64 runner
   - `aarch64-unknown-linux-gnu` on an Ubuntu 22.04 ARM64 runner
   - `x86_64-apple-darwin` on an Intel macOS runner
   - `aarch64-apple-darwin` on an Apple Silicon macOS runner

   Confirm current official GitHub runner labels rather than guessing. The
   Intel macOS label is the unstable one: `macos-13` is the last Intel image
   under the classic naming and GitHub has been retiring Intel images, with
   `macos-15-intel` introduced as the replacement. Check the current
   runner-images list for the supported Intel label before pinning it.

7. Smoke-tests each resulting binary with a safe command such as `runyte --version` or `runyte --help`.
8. Packages each target as a clearly named archive such as:

   `runyte-v0.1.0-x86_64-unknown-linux-gnu.tar.xz`

9. Places these files inside each archive:

   - the `runyte` executable
   - README.md
   - LICENSE
   - NOTICE
   - THIRD_PARTY_NOTICES.md
   - config.example.yaml

10. Preserves executable permissions and uses a versioned top-level directory inside each archive.
11. Uploads build results as intermediate Actions artifacts.
12. Uses a final Ubuntu publish job to:

   - download all four archives;
   - generate a combined `SHA256SUMS`;
   - create or update the GitHub Release associated with the exact tag;
   - upload the archives and checksum file;
   - use a clear title such as `Runyte 0.1.0`;
   - remain safe and reasonably idempotent when a failed release job is rerun.

Security and maintenance requirements:

- Give build jobs read-only repository permissions.
- Give only the final publishing job `contents: write`.
- Use the workflow-provided `GITHUB_TOKEN`; require no personal token.
- Treat the manual tag input as untrusted and quote/validate it before shell use.
- Pin third-party actions to full commit SHAs with comments naming their release versions.
- Prefer official GitHub actions and the installed `gh` CLI over an unnecessary release-upload action.
- Consider GitHub build-provenance attestations if they can be added cleanly with minimal permissions.
- Avoid mutable release inputs and accidental builds from the wrong commit.
- Do not add Windows artifacts.
- Do not add signing or macOS notarization yet; document that the binaries are unsigned if relevant.

Update documentation as part of the same change:

- Update README.md installation instructions to mention downloadable GitHub Release archives while retaining `cargo install runyte --locked`.
- Update context/reference/releasing.md so the documented order includes the binary workflow after a successful crates.io publish and tag push.
- Document how to run the manual v0.1.0 backfill after this workflow reaches `main`.
- Explain artifact names, supported targets, checksums, rerun behavior, and the fact that macOS artifacts are currently unsigned.
- Keep the release procedure unambiguous about which actions are manual and which GitHub performs automatically.

Validate the result:

- Check workflow YAML syntax.
- Use `actionlint` if it is already available; otherwise explain the validation used without installing unrelated global tools.
- Run any repository tests that cover packaging or release metadata.
- Run:
  - cargo fmt --check
  - cargo clippy --all-targets -- -D warnings
  - cargo test
- Review the final diff for excessive workflow permissions, unpinned actions, unsafe interpolation, incorrect target selection, and documentation inconsistencies.

When finished, report:

- files changed;
- exact triggers and target matrix;
- how v0.1.0 will be backfilled;
- security decisions;
- validation results;
- any remaining limitation, especially unsigned macOS binaries.
