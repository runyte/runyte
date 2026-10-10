# SPDX-License-Identifier: MPL-2.0
"""Register a Linux launcher or package the native editor with document preview."""
import argparse
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess

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


def preview_helper(binary, helper=None):
    helper = (helper or binary.with_name("runyte-preview-helper")).expanduser().resolve()
    if not helper.is_file() or not os.access(helper, os.X_OK):
        raise ValueError(f"preview helper is missing or not executable: {helper}; "
                         "build contrib/document-preview and pass --preview-helper")
    return helper


def bundle_linux(binary, destination, helper=None):
    helper = preview_helper(binary, helper)
    destination = destination.expanduser().absolute()
    if destination.exists():
        raise ValueError(f"package already exists: {destination}; choose a new output directory")
    destination.mkdir(parents=True)
    shutil.copy2(binary, destination / "runyte")
    shutil.copy2(helper, destination / "runyte-preview-helper")
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


def bundle_macos(binary, destination, helper=None):
    helper = preview_helper(binary, helper)
    destination = destination.expanduser().absolute()
    if destination.exists():
        raise ValueError(f"bundle already exists: {destination}; choose a new output directory")
    resources = destination / "Contents/Resources"
    executables = destination / "Contents/MacOS"
    executables.mkdir(parents=True)
    resources.mkdir()
    shutil.copy2(binary, executables / "runyte")
    shutil.copy2(helper, executables / "runyte-preview-helper")
    shutil.copyfile(HERE / "icons/Runyte.icns", resources / "Runyte.icns")
    repository = HERE.parents[1]
    for notice in ("LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"):
        shutil.copyfile(repository / notice, resources / notice)
    shutil.copytree(repository / "licenses", resources / "licenses")
    font_notices = resources / "assets/fonts/jetbrains-mono"
    font_notices.mkdir(parents=True)
    shutil.copyfile(repository / "assets/fonts/jetbrains-mono/README.md", font_notices / "README.md")
    launcher = executables / "runyte-window"
    launcher.write_text('#!/bin/sh\nPATH="${PATH:-/usr/bin:/bin:/usr/sbin:/sbin}:/opt/homebrew/bin:/usr/local/bin"\nexport PATH\nexec "$(dirname "$0")/runyte" --window --editor "$@"\n', encoding="utf-8")
    launcher.chmod(0o755)
    manifest = (repository / "Cargo.toml").read_text(encoding="utf-8")
    version = re.search(r'^version\s*=\s*"([^"]+)"', manifest, re.MULTILINE).group(1)
    with (destination / "Contents/Info.plist").open("wb") as output:
        plistlib.dump({
            "CFBundleIdentifier": APP_ID,
            "CFBundleName": "Runyte",
            "CFBundleDisplayName": "Runyte",
            "CFBundleExecutable": "runyte-window",
            "CFBundleIconFile": "Runyte.icns",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version,
            "NSHighResolutionCapable": True,
        }, output)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=("linux", "macos"))
    parser.add_argument("--binary", type=Path, required=True,
                        help="existing Runyte binary built with --features native")
    parser.add_argument("--data-dir", type=Path,
                        help="Linux XDG data directory (default: XDG_DATA_HOME or ~/.local/share)")
    parser.add_argument("--output", type=Path, help="new Linux package directory or macOS .app directory")
    parser.add_argument("--preview-helper", type=Path,
                        help="preview executable to bundle (default: beside --binary)")
    arguments = parser.parse_args()
    binary = arguments.binary.expanduser().resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("--binary must name an executable regular file")
    if arguments.platform == "linux":
        if arguments.output is not None:
            if arguments.data_dir is not None:
                parser.error("--output cannot be combined with --data-dir")
            print(bundle_linux(binary, arguments.output, arguments.preview_helper))
            return
        if arguments.preview_helper is not None:
            parser.error("--preview-helper requires --output")
        root = arguments.data_dir or Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
        print(install_linux(binary, root))
    else:
        if arguments.data_dir is not None or arguments.output is None:
            parser.error("macos requires --output and does not use --data-dir")
        print(bundle_macos(binary, arguments.output, arguments.preview_helper))


if __name__ == "__main__":
    main()
