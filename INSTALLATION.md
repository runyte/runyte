# Installing Runyte

Choose an edition, install it, then set up your shell. For editor commands and
configuration, see the [user guide](docs/user-guide/README.md).

- [Choose an edition](#choose-an-edition)
- [Terminal edition](#terminal-edition): Linux/macOS install script, archives, Cargo and [Windows](#windows)
- [Desktop edition](#desktop-edition): Linux archives and the macOS disk image
- [Build from source](#build-from-source): package names and system dependencies
- [Shell and external-editor setup](#shell-and-external-editor-setup)
- [Optional tools](#optional-tools)
- [Update or uninstall](#update-or-uninstall)

## Choose an edition

The **terminal edition** provides the editor, terminal UI, Git, language
services, integrated terminals, plugins and MCP. The **desktop edition** adds
the window, native clipboard, image/PDF viewing and document preview. It also
provides the terminal frontend, so you can use one desktop installation for both.

| | Terminal edition | Desktop edition |
| --- | --- | --- |
| Platforms | Linux, macOS, Windows 11 | Linux, macOS |
| Prebuilt archives | Linux/macOS x86-64 and ARM64; Windows x86-64 | Linux x86-64 and ARM64; macOS universal (Apple silicon and Intel) |
| Linux archive requirement | glibc 2.35 or newer | glibc 2.39 or newer; Vulkan driver and desktop libraries |
| Source-build toolchain | Rust 1.89 or newer and a C compiler | Current stable Rust, at least 1.92; platform dependencies below |
| Approximate executable size | 56 MB | 107 MB |

Sizes are stripped Linux x86-64 release measurements, not download sizes. Linux
release archives target glibc. Alpine and musl are not supported. The desktop
edition targets macOS 11 or later. Releases after v0.4.1 include a universal
`Runyte.app`, signed with a Developer ID and notarized by Apple, as a disk
image and as an archive. Windows requirements are [listed below](#windows).

**Both distributions install an executable named `runyte`.** Run
`runyte --version` to see its version and edition. `runyte --window` opens the
desktop window. Without `--window`, either edition runs in a terminal.

Cargo package names describe how the source is organized:

| Package | Role | Build output |
| --- | --- | --- |
| `runyte` | Shared editor core and terminal-edition executable | `target/release/runyte` |
| `runyte-desktop` | Desktop executable, including the terminal frontend, window and internal helpers | `target/release/runyte-desktop` |
| `runyte-native` | Library for the window, clipboard, media and PDF helpers | No standalone program |
| `runyte-preview` | Library for the document-preview engine | No standalone program |

The desktop build has a different filename so both editions can coexist in
`target/`. Desktop packaging installs that executable as `runyte`. You do not
install the native or preview libraries separately.

## Terminal edition

### Install script (Linux and macOS)

On x86-64 or ARM64 Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/runyte/runyte/main/install.sh | sh
```

Run the same command again to update. The script selects the release for your
machine, verifies its SHA-256 checksum, and installs `runyte` in `~/.local/bin`
without sudo. It also creates `runed -> runyte` unless an unrelated `runed`
already exists. It does not change configuration or shell startup files. A
failed download, checksum or extraction leaves the existing editor intact. If
the `runyte` it would replace is the desktop edition, the script updates the
desktop edition instead. `--terminal` switches that installation to the
terminal edition. `--desktop` installs the [desktop
edition](#install-script-desktop-edition).

You need `curl`, `tar` with xz support, `sha256sum` or `shasum`, and standard Unix
utilities. Some distributions package xz separately as `xz-utils` or `xz`.
Linux requires glibc 2.35 or newer. The macOS terminal binaries are unsigned
and unnotarized. The script does not change Gatekeeper settings.

To inspect the script and select a version or install directory:

```sh
curl -fsSL https://raw.githubusercontent.com/runyte/runyte/main/install.sh -o install.sh
less install.sh
sh install.sh --version X.Y.Z --install-dir "$HOME/.local/bin"
```

Replace `X.Y.Z` with a release version. Leave out `--version` for the latest
release. Use an absolute install directory and keep that directory when
updating. Historical archive names are supported. The installer refuses a
symlink or directory at the destination `runyte` path. The exception is an
absolute link into a `Runyte.app`, such as the one it creates for the macOS
desktop edition.

### Release archives

Download the archive for your edition, OS and CPU, together with `SHA256SUMS`,
from [GitHub Releases](https://github.com/runyte/runyte/releases).

| Platform | Terminal archive name |
| --- | --- |
| Linux x86-64 | `runyte-terminal-vX.Y.Z-x86_64-unknown-linux-gnu.tar.xz` |
| Linux ARM64 | `runyte-terminal-vX.Y.Z-aarch64-unknown-linux-gnu.tar.xz` |
| macOS Intel | `runyte-terminal-vX.Y.Z-x86_64-apple-darwin.tar.xz` |
| macOS Apple silicon | `runyte-terminal-vX.Y.Z-aarch64-apple-darwin.tar.xz` |
| Windows x86-64 | `runyte-terminal-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

Replace `X.Y.Z` with the release version. Older releases use the `runyte-`
prefix instead of `runyte-terminal-`.

Compute the downloaded archive's digest with `sha256sum ARCHIVE` on Linux,
`shasum -a 256 ARCHIVE` on macOS, or `Get-FileHash -Algorithm SHA256 ARCHIVE`
in PowerShell. Compare the complete digest with that archive's entry in
`SHA256SUMS`. These checks show the file matches the release manifest. They are
not independent signatures.

Extract the archive into a directory you intend to keep. It contains `runyte`,
`runed`, configuration examples and licence material. Add that directory to
`PATH`, or link the executables into a directory already on `PATH`. On Windows,
use the [Windows steps](#windows). On macOS, terminal executables are unsigned
and unnotarized.

### Install with Cargo

Install [Rust through rustup](https://rustup.rs/) and a C compiler. You need
Rust 1.89 or newer. The C compiler builds the bundled Tree-sitter grammars. On
Windows, use the MSVC toolchain described [below](#windows).

```sh
cargo install runyte --locked
```

Keep `--locked` to use the dependency versions tested with the release. Cargo
installs the terminal edition as `~/.cargo/bin/runyte` by default. It does not
install the desktop edition or a `runed` link. See [shell setup](#shell-and-external-editor-setup)
to add the link.

### Windows

The terminal edition supports x86-64 Windows 11 24H2 or later in Windows
Terminal. Download and verify the Windows ZIP, extract it to a stable location
such as `C:\Tools\Runyte`, and add that directory to your user `PATH`. Open a new
terminal and run `runyte --version`.

The executables are unsigned. The ZIP includes `runyte.exe` and `runed.exe`, but
does not bundle the x64 Microsoft Visual C++ runtime (`VCRUNTIME140.dll`). If
needed, install the [x64 Visual C++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist).

Source builds and Cargo installation need Rust 1.89 or newer with the MSVC
toolchain, Visual Studio Build Tools with C++ support, and the Windows SDK.
From a source checkout:

```powershell
cargo build --release --locked
.\target\release\runyte.exe --editor README.md
```

See [Windows support](docs/user-guide/install-and-run.md#windows-support) for feature differences
and validation limits.

## Desktop edition

The desktop edition supports Linux and macOS. There is no Windows desktop edition.

### Install script (desktop edition)

The [install script](#install-script-linux-and-macos) installs the desktop
edition with `--desktop`:

```sh
curl -fsSL https://raw.githubusercontent.com/runyte/runyte/main/install.sh | sh -s -- --desktop
```

Later runs of the same script, with or without `--desktop`, update the desktop
edition, because the script keeps the edition it finds installed. Pass
`--terminal` to switch back.

- **Linux:** the script checks for glibc 2.39 or newer and the window's XCB and
  XKB libraries (in the loader cache or `LD_LIBRARY_PATH`) before downloading.
  It then installs `runyte` and `runed` in `~/.local/bin`, as for the terminal
  edition. With Python 3 it also registers the launcher and icons for your
  desktop account, as described below. Without Python 3, `runyte --window`
  still works.
- **macOS 11 or later:** the script checks that the downloaded app has the
  Runyte release team's Developer ID signature and passes Gatekeeper. It
  installs `Runyte.app` in `~/Applications` and links `~/.local/bin/runyte` to
  the app's executable. To choose another directory, use
  `--app-dir /absolute/path`, for example `--app-dir /Applications`. Updates
  replace the app in the directory it was installed in. If the new app cannot
  be put in place, the previous one is restored. Switching back with
  `--terminal` leaves the app in place.

If the script cannot tell which edition an existing `runyte` is, because its
`--version` fails, it asks you to choose with `--desktop` or `--terminal`.

### Linux release archives

Download `runyte-desktop-vX.Y.Z-x86_64-unknown-linux-gnu.tar.xz` or
`runyte-desktop-vX.Y.Z-aarch64-unknown-linux-gnu.tar.xz` from
[GitHub Releases](https://github.com/runyte/runyte/releases). Verify the checksum
as described under [release archives](#release-archives).

The archives require glibc 2.39 or newer (for example, Ubuntu 24.04), a graphical
desktop and a working Vulkan driver. The window uses Fontconfig, FreeType, XCB,
XKB and Wayland libraries. The [Linux dependency lists](#linux-desktop-dependencies)
below include these libraries together with the tools needed for source builds.

Extract into a permanent directory, then run these commands from that directory:

```sh
./runyte --version
./runyte --window --editor
python3 contrib/packaging/package.py linux --binary ./runyte
```

The last command registers the launcher and icons for your desktop account. It
needs Python 3. Registration supplies the window icon on Wayland. It records
the executable's absolute path, so register again if you move the installation.
The launcher opens your account home in the configured mode. IDE and mux use
the home workspace with integrated terminals. In these modes, file arguments
choose the first file’s directory for workspace discovery. Editor mode keeps
ordinary file opening. To select persistent mode explicitly, launch
`runyte --window --mux` from the project directory instead.

Add the extracted directory to `PATH`, or link its `runyte` and `runed` into
`~/.local/bin`. Keep the package's documentation, licences and registration
files. PDF and document-preview helpers are included in the editor executable.
There is no separate helper to install.

### macOS

Download `Runyte-X.Y.Z.dmg` from [GitHub
Releases](https://github.com/runyte/runyte/releases). It runs on Apple silicon
and Intel Macs with macOS 11 or later. Open it, drag `Runyte` onto
`Applications`, then eject the image. Launch Runyte from Finder, Launchpad or
Spotlight. It opens your home directory in the configured mode. IDE and mux use
the home workspace with integrated terminals. `mode: editor` keeps editor mode.
The app is signed with a Developer ID and notarized, so macOS asks only its
usual first-launch confirmation. To update, replace `/Applications/Runyte.app`
with the newer one the same way.

The same app is published as
`runyte-desktop-vX.Y.Z-universal-apple-darwin.tar.xz`, listed in `SHA256SUMS`.
The [install script](#install-script-desktop-edition) installs it with
`--desktop` and puts `runyte` on your `PATH`.

To build the app yourself instead, use the [macOS build dependencies](#macos-desktop-dependencies) and
[desktop build command](#build-from-source). The [packaging guide](contrib/packaging/README.md#macos)
explains how to create a universal unsigned `Runyte.app` with its launcher and
icons. An unsigned local bundle is not a signed release download.

For an app installed in `/Applications`, optional command-line links are:

```sh
mkdir -p "$HOME/.local/bin"
ln -s /Applications/Runyte.app/Contents/MacOS/runyte "$HOME/.local/bin/runyte"
ln -s /Applications/Runyte.app/Contents/MacOS/runyte "$HOME/.local/bin/runed"
```

Use a free destination for each link, or deliberately replace an older
installation first. Add `~/.local/bin` to `PATH` as described below.

## Build from source

Clone the repository and enter it, or use an existing checkout:

```sh
git clone https://github.com/runyte/runyte.git
cd runyte
```

To build a particular release, check out its `vX.Y.Z` tag before building.
The checkout's `rust-toolchain.toml` pins the development toolchain, which rustup
selects automatically. The terminal package's minimum supported Rust version is
1.89. Desktop builds require at least 1.92. Install the platform dependencies
below before building the desktop edition.

Choose one build:

```sh
# Terminal edition
cargo build --release --locked
./target/release/runyte --editor README.md

# Desktop edition (Linux or macOS)
cargo build --release --locked -p runyte-desktop
./target/release/runyte-desktop --window --mux
```

`./build.sh --release --locked` wraps `cargo build --bins` for the terminal
edition. It forwards additional Cargo arguments. Omit `--release` for a debug
build, whose executable is under `target/debug/`.

The Cargo workspace shares one lockfile. Resolving it can fetch the pinned
Blitz/Taffy Git dependencies even for a terminal-only build. The terminal
package does not compile or link those engines. The published crates.io package
has no desktop dependencies.

### Linux desktop dependencies

These lists include build tools and window libraries. A terminal-only source
build needs Rust and a C toolchain, without the window dependencies.

**Debian / Ubuntu:**

```sh
sudo apt update
sudo apt install build-essential pkg-config cmake clang \
  libfontconfig1-dev libfreetype6-dev libxcb1-dev libx11-xcb-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libssl-dev \
  libvulkan1 mesa-vulkan-drivers
```

**Fedora:**

```sh
sudo dnf install gcc gcc-c++ pkgconf-pkg-config cmake clang \
  fontconfig-devel freetype-devel libxcb-devel libX11-devel \
  libxkbcommon-devel libxkbcommon-x11-devel wayland-devel openssl-devel \
  vulkan-loader mesa-vulkan-drivers
```

**Arch Linux:**

```sh
sudo pacman -Syu --needed base-devel pkgconf cmake clang fontconfig freetype2 \
  libxcb libx11 libxkbcommon libxkbcommon-x11 wayland openssl \
  vulkan-icd-loader
```

You also need the Vulkan driver appropriate for your GPU. The Debian/Ubuntu
and Fedora commands include Mesa's drivers. NVIDIA users should use their
distribution's matching NVIDIA driver. On Arch, install `vulkan-radeon` for AMD
or `vulkan-intel` for Intel. For other distributions, consult the
[upstream GPUI/Linux build guidance](https://zed.dev/docs/development/linux).

### macOS desktop dependencies

Install [Xcode](https://apps.apple.com/us/app/xcode/id497799835), launch it once,
and install its macOS components. GPUI compiles Metal shaders, so desktop builds
need full Xcode in addition to the command-line tools. With
[Homebrew](https://brew.sh/) installed:

```sh
xcode-select --install  # If the command-line tools are not installed yet.
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
sudo xcodebuild -license
xcodebuild -downloadComponent MetalToolchain
brew install cmake pkgconf
```

If the build reports `cannot execute tool metal due to missing Metal Toolchain`,
run `xcodebuild -downloadComponent MetalToolchain`, then retry. See the
[upstream macOS build guidance](https://zed.dev/docs/development/macos) for Xcode
setup details. Terminal-only builds need the command-line C toolchain, not the
Metal toolchain.

### Install a source build

For the terminal edition, Cargo can build and install from the checkout:

```sh
cargo install --path . --locked
```

For the desktop edition on Linux or macOS, install the release executable under
the distribution name `runyte`. These commands choose `~/.local/bin`:

```sh
mkdir -p "$HOME/.local/bin"
install -m 0755 target/release/runyte-desktop "$HOME/.local/bin/runyte"
```

This destination becomes your desktop installation. Check for an existing copy
before replacing it. The desktop edition also provides terminal operation. Add
a `runed` link if wanted, as described below.

On Linux, register the installed executable and icons from the checkout:

```sh
python3 contrib/packaging/package.py linux --binary "$HOME/.local/bin/runyte"
```

Registration adds a desktop launcher. It does not copy the executable. To build
a complete directory package or macOS app instead, use the
[packaging guide](contrib/packaging/README.md).

## Shell and external-editor setup

### PATH and executable selection

On Linux and macOS, add your install directory to `PATH` in `~/.bashrc` or
`~/.zshrc` if needed:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Cargo normally uses `~/.cargo/bin`. Rustup's shell setup makes it available.
Check which copy your shell finds, especially if you have used more than one
installation method:

```sh
command -v runyte
runyte --version
```

On Windows, use `Get-Command runyte` in PowerShell. If `--window` is refused,
check that the selected executable reports the desktop edition.

### Codex, Claude Code and other external-editor requests

Set both variables in `~/.bashrc` or `~/.zshrc`:

```sh
export EDITOR='runyte --wait'
export VISUAL='runyte --wait'
```

In PowerShell, put the following in your `$PROFILE`:

```powershell
$env:EDITOR = 'runyte --wait'
$env:VISUAL = 'runyte --wait'
```

For editing inside the parent Runyte instance, start a **persistent session**
in your project, then launch the agent inside an integrated terminal:

```sh
runyte --mux           # terminal frontend, either edition
runyte --window --mux  # window frontend, desktop edition
```

Codex/Claude Code's `Ctrl-g` external-editor request then opens the file as a
buffer in that persistent session. `:wbc` writes and closes the buffer, restores
the terminal and lets the caller resume. `:w` alone saves but keeps the request
pending. Standalone IDE mode, including `--window` without `--mux` under the
default configuration, does not support this parent-routed workflow.

Use `--wait` in `EDITOR`, without adding `--window`: it routes the request to
the existing persistent session. Either edition can send that request, but all
clients and the host must use the same Runyte version. Using one desktop
installation for both avoids selecting different versions.

If you run directly from a build directory, use its actual executable path:

```sh
export EDITOR='/absolute/path/to/runyte/target/release/runyte-desktop --wait'
export VISUAL="$EDITOR"
```

Replace the example path. If it contains spaces, quote the executable path
inside the variable too. Shell aliases are not a substitute for an executable
on `PATH`. Start a new shell and restart already-running agents after changing
these variables. Existing processes retain their old environment.

For Git specifically, you can also set:

```sh
git config --global core.editor 'runyte --wait'
```

See [external editor requests](docs/user-guide/workspaces-and-persistent-sessions.md#external-editor-requests---wait)
for completion, cancellation and ordinary-shell behavior.

### Quick edits and optional shell integration

`runed` is the same executable launched in standalone editor mode, equivalent
to `runyte --editor`. The install script and release archives provide it. After
Cargo installation, add it on Unix with:

```sh
ln -s runyte "$HOME/.cargo/bin/runed"
```

For a desktop source build installed in `~/.local/bin`, use that directory
instead. On Windows, `runed.exe` is a copy of `runyte.exe`.

For standalone edits you can choose `EDITOR=runed` and `VISUAL=runed` instead
of the persistent-session setup above. For system files on Unix, use
`SUDO_EDITOR=runed` with `sudoedit`.

Optional conveniences are `alias ru=runyte` in Bash/Zsh or `Set-Alias ru runyte`
in PowerShell. To let `:quit-here` change the calling shell's directory, use the
wrappers in the [shell directory guide](docs/user-guide/workspaces-and-persistent-sessions.md#change-the-shell-directory-on-exit).
On Windows, keep [runyte.ps1](contrib/runyte.ps1) at a stable location and
dot-source it from `$PROFILE`. The wrapper is tested with Windows PowerShell
5.1. PowerShell 7 has not been validated.

## Optional tools

- **Git:** install `git` and make it available on `PATH` for Git features.
- **Language servers:** install the servers for your languages separately;
  see [language-server setup](docs/lsp/README.md).
- **Terminal clipboard on Linux:** install `wl-clipboard`, `xclip`, or `xsel`.
  The desktop edition also supplies a native clipboard integration.
- **PDF fallback:** Hayro is built into the desktop executable. Poppler is
  optional, for documents the built-in renderer can't handle. Install
  `poppler-utils` on Debian/Ubuntu or Fedora, or `poppler` on Arch or through
  Homebrew. Its `pdfinfo`, `pdftoppm`, and `pdftotext` commands must all be on
  `PATH`.
- **Fonts:** the terminal edition uses your terminal emulator's font. The
  desktop window and its document preview include JetBrainsMono Nerd Font. You
  don't need to install a font. See its [licences and
  attribution](licenses/jetbrains-mono/README.md).

Window controls, media navigation and document preview are covered in the
[desktop user guide](docs/user-guide/files-and-buffers.md#desktop-edition).

## Update or uninstall

Update the same way you installed. Run the terminal install script or
`cargo install runyte --locked` again, replace a manually downloaded package,
or rebuild and install from source. The install script keeps the edition it
finds, so it updates a desktop installation it made, or a desktop `runyte`
executable in its install directory. A `runyte` link into a `Runyte.app`,
including the disk-image link shown under [macOS](#macos), counts as a desktop
installation. The script updates that app where it is and makes `runed` a link
to `runyte`. It refuses other symlinks, such as one to a manually extracted
Linux archive.

Save your work and restart running editors and persistent hosts after an update
so every process uses the same version. If you have multiple installations,
check `command -v runyte` and `runyte --version` again. Re-register a Linux desktop
launcher if its executable path changed.

To uninstall a script installation, remove its `runyte` executable and `runed`
link from the install directory. For the macOS desktop edition, also remove its
`Runyte.app`. For Cargo, run `cargo uninstall runyte` and remove a manually
created `runed` link. For a manual package or app, remove its directory and any
command-line links you created. If you registered a Linux launcher, remove
`com.runyte.Runyte.desktop` from
`${XDG_DATA_HOME:-~/.local/share}/applications` and its `com.runyte.Runyte`
icons from that data directory's `icons/hicolor` tree. Remove shell profile
entries you added if you no longer need them. Configuration is kept separately.
Its locations are in the [configuration
guide](docs/user-guide/configuration.md#configuration).
