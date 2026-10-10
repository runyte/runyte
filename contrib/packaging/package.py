# SPDX-License-Identifier: MPL-2.0
"""Register a Linux launcher or package the native editor with document preview."""
import argparse
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile

APP_ID = "com.runyte.Runyte"
HERE = Path(__file__).resolve().parent


def desktop_argument(value):
    """Quote one Exec argument using the desktop-entry string and Exec rules."""
    if any(character in value for character in "\r\n\0"):
        raise ValueError("desktop executable path must not contain control characters")
    value = value.replace("%", "%%")
    for character in ("\\", '"', "`", "$"):
        value = value.replace(character, "\\" + character)
    # Desktop-entry string unescaping happens before Exec argument unquoting.
    return '"' + value.replace("\\", "\\\\") + '"'


def install_linux(binary, data_dir, refresh=True):
    data_dir = data_dir.expanduser().resolve()
    applications = data_dir / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    for size in (16, 32, 64, 128, 256, 512, 1024):
        directory = data_dir / "icons/hicolor" / f"{size}x{size}" / "apps"
        directory.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(HERE / "icons" / f"runyte-{size}.png", directory / f"{APP_ID}.png")
    scalable = data_dir / "icons/hicolor/scalable/apps"
    scalable.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(HERE / "icons" / f"{APP_ID}.svg", scalable / f"{APP_ID}.svg")
    desktop = applications / f"{APP_ID}.desktop"
    desktop.write_text(f'''[Desktop Entry]
Type=Application
Version=1.0
Name=Runyte
Comment=Modal text editor
Exec={desktop_argument(str(binary))} --window --editor %F
Icon={APP_ID}
Terminal=false
Categories=Development;TextEditor;
MimeType=text/plain;
StartupWMClass={APP_ID}
''', encoding="utf-8")
    # Notify desktop/icon watchers even when files replace existing paths.
    os.utime(data_dir / "icons/hicolor", None)
    if refresh:
        for command, directory in (("update-desktop-database", applications),
                                   ("gtk-update-icon-cache", data_dir / "icons/hicolor")):
            executable = shutil.which(command)
            if executable:
                flags = ["-f", "-t"] if command == "gtk-update-icon-cache" else []
                subprocess.run([executable, *flags, str(directory)], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return desktop


def bundle_linux(binary, destination):
    destination = destination.expanduser().absolute()
    if destination.exists():
        raise ValueError(f"package already exists: {destination}; choose a new output directory")
    destination.mkdir(parents=True)
    shutil.copy2(binary, destination / "runyte")
    (destination / "runed").symlink_to("runyte")
    repository = HERE.parents[1]
    for name in ("README.md", "LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md", "config.example.yaml"):
        shutil.copyfile(repository / name, destination / name)
    shutil.copytree(repository / "licenses", destination / "licenses")
    (destination / "docs").mkdir()
    shutil.copyfile(repository / "docs/user-guide.md", destination / "docs/user-guide.md")
    integration = destination / "contrib/packaging"
    integration.mkdir(parents=True)
    for name in ("package.py", "README.md"):
        shutil.copyfile(HERE / name, integration / name)
    shutil.copytree(HERE / "icons", integration / "icons")
    return destination


def check_universal(binary):
    architectures = subprocess.check_output(["lipo", "-archs", str(binary)], text=True).split()
    if set(architectures) != {"arm64", "x86_64"}:
        raise ValueError(f"{binary} must contain arm64 and x86_64, found {architectures}")


def bundle_macos(binary, destination, launcher):
    destination = destination.expanduser().absolute()
    if destination.exists():
        raise ValueError(f"bundle already exists: {destination}; choose a new output directory")
    check_universal(binary)
    check_universal(launcher)
    resources = destination / "Contents/Resources"
    executables = destination / "Contents/MacOS"
    executables.mkdir(parents=True)
    resources.mkdir()
    shutil.copy2(binary, executables / "runyte")
    shutil.copyfile(HERE / "icons/Runyte.icns", resources / "Runyte.icns")
    repository = HERE.parents[1]
    for notice in ("LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"):
        shutil.copyfile(repository / notice, resources / notice)
    shutil.copytree(repository / "licenses", resources / "licenses")
    font_notices = resources / "assets/fonts/jetbrains-mono"
    font_notices.mkdir(parents=True)
    shutil.copyfile(repository / "assets/fonts/jetbrains-mono/README.md", font_notices / "README.md")
    shutil.copy2(launcher, executables / "RunyteLauncher")
    (executables / "runed").symlink_to("runyte")
    manifest = (repository / "Cargo.toml").read_text(encoding="utf-8")
    version = re.search(r'^version\s*=\s*"([^"]+)"', manifest, re.MULTILINE).group(1)
    with (destination / "Contents/Info.plist").open("wb") as output:
        plistlib.dump({
            "CFBundleIdentifier": APP_ID,
            "CFBundleName": "Runyte",
            "CFBundleDisplayName": "Runyte",
            "CFBundleExecutable": "RunyteLauncher",
            "LSMinimumSystemVersion": "11.0",
            "NSHumanReadableCopyright": "Copyright Runyte contributors. Licensed under MPL-2.0.",
            "LSApplicationCategoryType": "public.app-category.developer-tools",
            "CFBundleIconFile": "Runyte.icns",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version,
            "NSHighResolutionCapable": True,
        }, output)
    return destination


def disk_image(app, output):
    app = app.expanduser().resolve(strict=True)
    output = output.expanduser().absolute()
    if output.exists() or output.is_symlink():
        raise ValueError(f"disk image already exists: {output}")
    with (app / "Contents/Info.plist").open("rb") as source:
        version = plistlib.load(source)["CFBundleShortVersionString"]
    with tempfile.TemporaryDirectory(prefix="runyte-dmg-") as temporary:
        stage = Path(temporary)
        shutil.copytree(app, stage / "Runyte.app", symlinks=True)
        (stage / "Applications").symlink_to("/Applications")
        shutil.copyfile(HERE.parents[1] / "README.md", stage / "README.md")
        subprocess.run(["hdiutil", "create", "-volname", f"Runyte {version}",
                        "-srcfolder", str(stage), "-format", "UDZO", str(output)], check=True)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=("linux", "macos", "dmg"))
    parser.add_argument("--app", type=Path, help="existing Runyte.app for a disk image")
    parser.add_argument("--binary", type=Path,
                        help="existing runyte-desktop binary")
    parser.add_argument("--launcher", type=Path, help="universal macOS runyte-app-launcher")
    parser.add_argument("--data-dir", type=Path,
                        help="Linux XDG data directory (default: XDG_DATA_HOME or ~/.local/share)")
    parser.add_argument("--output", type=Path, help="new Linux package directory or macOS .app directory")
    arguments = parser.parse_args()
    if arguments.platform == "dmg":
        if arguments.app is None or arguments.output is None:
            parser.error("dmg requires --app and --output")
        if any(value is not None for value in (arguments.binary, arguments.launcher, arguments.data_dir)):
            parser.error("dmg does not use --binary, --launcher or --data-dir")
        print(disk_image(arguments.app, arguments.output))
        return
    if arguments.binary is None or arguments.app is not None:
        parser.error("linux and macos require --binary and do not use --app")
    binary = arguments.binary.expanduser().resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("--binary must name an executable regular file")
    if arguments.platform == "linux":
        if arguments.launcher is not None:
            parser.error("--launcher is only used for macos")
        if arguments.output is not None:
            if arguments.data_dir is not None:
                parser.error("--output cannot be combined with --data-dir")
            print(bundle_linux(binary, arguments.output))
            return
        root = arguments.data_dir or Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
        print(install_linux(binary, root))
    else:
        if arguments.data_dir is not None or arguments.output is None:
            parser.error("macos requires --output and does not use --data-dir")
        if arguments.launcher is None:
            parser.error("macos requires --launcher")
        launcher = arguments.launcher.expanduser().resolve(strict=True)
        if not launcher.is_file() or not os.access(launcher, os.X_OK):
            parser.error("--launcher must name an executable regular file")
        print(bundle_macos(binary, arguments.output, launcher))


if __name__ == "__main__":
    main()
