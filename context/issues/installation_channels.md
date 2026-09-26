# Add quick install and package manager options

Installation currently requires a manual download of a GitHub Release archive,
`cargo install runyte --locked`, or a build from a clone. The release workflow
already publishes versioned archives and `SHA256SUMS` for x86-64 and ARM64
Linux and macOS, and x86-64 Windows. There is no documented one-command
install using a shell script, Homebrew, or WinGet.

Provide these three installation paths first:

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
