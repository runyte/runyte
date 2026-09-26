# Add quick install and package manager options

The curl installation path is implemented in the repository-root `install.sh`.
The same script installs and updates Runyte on x86-64 and ARM64 Linux/macOS,
verifies the selected release archive against `SHA256SUMS`, and replaces the
executable in `$HOME/.local/bin` or an explicit `--install-dir`.
`--version` selects a fixed release; omitting it selects the latest release.
The README and user guide document the command, runtime requirements, upgrade
behavior, and script review. The public command becomes available when the
script reaches `main`. Offline acceptance lives in
`tests/installer/test_install.py` and runs on Linux and macOS in CI.

Homebrew and WinGet remain open and are outside the curl implementation scope.

## Original requirements

At the time of the report, installation required a manual download of a GitHub
Release archive, `cargo install runyte --locked`, or a build from a clone. The
release workflow already published versioned archives and `SHA256SUMS` for
x86-64 and ARM64 Linux and macOS, and x86-64 Windows. There was no documented
one-command install using a shell script, Homebrew, or WinGet.

The initial installation paths requested were:

- A `curl` command on Linux, and preferably macOS, that obtains a reviewed
  installer script. The script should select a supported OS and architecture,
  download the matching versioned release archive, verify it against the
  published checksum, and install the executable to a documented location.
  Unsupported systems and failed downloads or checks must fail clearly.
- A Homebrew package for supported macOS architectures, with a documented
  `brew install` command. Decide whether to publish through an upstream tap
  first or submit a formula to Homebrew core, and define how its version and
  hashes are updated after each release.
- A WinGet package for supported Windows x86-64 releases, with a documented
  `winget install` command. Its manifest needs a versioned installer URL and
  hash, and should account for the Visual C++ runtime requirement documented
  for the current Windows ZIP. Define how new versions are submitted after the
  release artifacts are available.

Keep the installation commands, supported-platform claims, verification, and
upgrade instructions aligned with the actual published packages. These
distribution steps must fit the existing release order: the version-only
release commit comes first, then crates.io publication, then the tag and its
binary workflow. Debian/Ubuntu and Arch packages are further options, but the
script, Homebrew, and WinGet are the initial priority.
