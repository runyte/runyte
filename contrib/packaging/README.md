# Native desktop integration

Desktop packages contain one editor executable with PDF and document preview
helpers linked into it. Build it from the checkout:

```sh
cargo build --release --locked -p runyte-desktop
```

The packaging tool uses this existing binary. The desktop crates share the
workspace lockfile. Ordinary terminal builds do not compile the preview engine.

## Linux

The release workflow builds an additional
`runyte-desktop-v<version>-x86_64-unknown-linux-gnu.tar.xz` archive on Ubuntu
24.04 (glibc 2.39 or newer). This desktop artifact is separate from the terminal
archives and the curl installer. It requires the native runtime libraries and
a working Vulkan driver described in the repository README. Publication is
limited to Linux x86-64 until other platforms have window/preview acceptance.

Extract the archive to its final location and run `./runyte --window --editor`. Preview runs through the packaged executable. From that extracted directory, register a desktop launcher:

```sh
python3 contrib/packaging/package.py linux --binary ./runyte
```

To construct the same directory layout from a checkout:

```sh
python3 contrib/packaging/package.py linux --binary target/release/runyte-desktop \
  --output /tmp/runyte-desktop
```

`--output` must name a new directory. The package includes the editor,
`runed`, notices, the user guide, icons and the desktop registration script.

Register the launcher and icon for the current desktop account:

```sh
python3 contrib/packaging/package.py linux --binary target/release/runyte-desktop
```

The helper writes `com.runyte.Runyte.desktop` under
`${XDG_DATA_HOME:-~/.local/share}/applications` and SVG/PNG icons into the
matching `icons/hicolor` directories. It refreshes desktop/icon caches when
the standard cache tools are installed. `--data-dir <directory>` selects a
different installation root, useful for packaging and isolated checks. The
launcher stores the binary's absolute path; register it again after moving
the binary. It opens `--window --editor`, accepting file arguments. Running
`runyte --window` directly in a workspace still uses the ordinary IDE mode.

On Wayland, the desktop finds the icon through this installed entry and the
window's matching application ID. Install it before opening a fresh window.
GPUI 0.2.2 has no Wayland toplevel-icon setter, so an unregistered executable
can still receive the compositor's generic icon. Desktop grouping and the
places where icons appear remain compositor policy.

On X11, Runyte also sets `_NET_WM_ICON` for its own window, independently of
desktop registration. The bounded, single startup scan matches both process
ID and window class. It does not poll, write desktop files or alter another
application's icon. Failure leaves the editor usable.

## macOS

On a Mac, build both slices with the macOS 11 deployment floor, then combine
both the editor and its native launcher:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --locked \
    -p runyte-desktop --features app-launcher --bins --target "$target"
done
mkdir -p target/universal
for binary in runyte-desktop runyte-app-launcher; do
  lipo -create "target/aarch64-apple-darwin/release/$binary" \
    "target/x86_64-apple-darwin/release/$binary" -output "target/universal/$binary"
done
python3 contrib/packaging/package.py macos --binary target/universal/runyte-desktop \
  --launcher target/universal/runyte-app-launcher --output /tmp/Runyte.app
python3 contrib/packaging/check_package.py /tmp/Runyte.app
```

Both inputs must contain ARM64 and x86-64 slices. Acceptance checks both
executables' deployment floor with `vtool` and rejects non-system dynamic
libraries with `otool`, including accidental Homebrew links.

The helper refuses to replace an existing bundle. It copies the editor into `Contents/MacOS`, alongside the
native `Runyte` launcher, relative `runed` link, ICNS icon and project/dependency/font notices. The launcher selects
`--window --editor` and preserves `PATH`, adding `/opt/homebrew/bin` and
`/usr/local/bin` so the usual Poppler installations remain discoverable from
Finder. Fonts are embedded in the binary. The bundle has identifier
`com.runyte.Runyte`, `CFBundleExecutable = Runyte`, a macOS 11 minimum, and
declares its icon through `CFBundleIconFile`. A leading Finder `-psn_*` argument
is discarded; other arguments remain intact.

This is an unsigned local-development bundle, not a notarized distribution.
Bundle structure and resources are tested on Linux; Finder/Dock icon behavior
and the native macOS window remain to be verified on macOS. Running the bare
CLI executable does not provide the bundle's Finder icon. Finder file-open
events are not integrated; open documents inside the editor or pass paths
when invoking the bundled binary from a terminal.

## Artwork and checks

`logo/runyte_logo.svg` remains the source of the mark. Generated desktop icons
retain its charcoal fill and geometry, with a light rounded backplate to
preserve contrast on dark panels. Regenerate with Python and `rsvg-convert`:

```sh
python3 contrib/packaging/render_icons.py
python3 -m unittest discover -s contrib/packaging -p 'test_*.py'
cargo test -p runyte-native --lib icon
```

`check_package.py` runs the real document engine from the packaged helper:

```sh
python3 contrib/packaging/check_package.py /tmp/runyte-desktop
# Use a dedicated X11 display: the test owns its clipboard.
xvfb-run -a -s '-screen 0 3200x1800x24' \
  python3 contrib/packaging/check_package.py /tmp/runyte-desktop --window
```

The release job runs this against the extracted archive before upload. The window
check invokes `:preview` through the packaged editor, exercising internal helper
launch, rendering, scrolling, selection/copy and return to source.
Native CI also constructs a macOS bundle and runs its headless engine checks;
this does not establish macOS window acceptance.

The X11 acceptance harness in `tests/native_window.py` checks the real
`WM_CLASS` and `_NET_WM_ICON` properties, including the expected ARGB colors.
Run it only on an isolated display, as described in the native-window record.

The platform contracts are the [desktop entry specification](https://specifications.freedesktop.org/desktop-entry/latest-single/),
the [XDG application ID](https://wayland.app/protocols/xdg-shell#xdg_toplevel:request:set_app_id),
the [X11 icon property](https://specifications.freedesktop.org/wm/latest/ar01s05.html)
and Apple's [bundle structure](https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFBundles/BundleTypes/BundleTypes.html).
