# SPDX-License-Identifier: MPL-2.0
"""Exercise compiled desktop executables from an extracted package or app bundle."""
import argparse
import os
import re
import plistlib
import package
from pathlib import Path
import subprocess
import sys
import tempfile


def check_layout(directory, repository):
    """Reject missing files and accidental extra executables in release packages."""
    def files(root):
        return {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file() or p.is_symlink()}
    licenses = {'licenses/' + name for name in files(repository / 'licenses')}
    if (directory / 'Contents/MacOS').is_dir():
        expected = {'Contents/Info.plist', 'Contents/MacOS/runyte', 'Contents/MacOS/Runyte', 'Contents/MacOS/runed'}
        link = directory / 'Contents/MacOS/runed'
        if not link.is_symlink() or os.readlink(link) != 'runyte':
            raise ValueError('bundle requires runed -> runyte')
        expected |= {'Contents/Resources/' + name for name in licenses | {
            'Runyte.icns', 'LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md',
            'assets/fonts/jetbrains-mono/README.md'}}
    else:
        expected = licenses | {'runyte', 'runed', 'README.md', 'LICENSE', 'NOTICE',
            'THIRD_PARTY_NOTICES.md', 'config.example.yaml', 'docs/user-guide.md',
            'contrib/packaging/package.py', 'contrib/packaging/README.md'}
        expected |= {'contrib/packaging/icons/' + name for name in files(repository / 'contrib/packaging/icons')}
        if not (directory / 'runed').is_symlink() or os.readlink(directory / 'runed') != 'runyte':
            raise ValueError('package requires runed -> runyte')
    actual = files(directory)
    if actual != expected:
        raise ValueError(f'package files differ: missing={sorted(expected-actual)}, extra={sorted(actual-expected)}')


def check_macos_binaries(directory):
    with (directory / "Contents/Info.plist").open("rb") as source:
        info = plistlib.load(source)
    if info.get("CFBundleExecutable") != "Runyte" or info.get("LSMinimumSystemVersion") != "11.0":
        raise ValueError("bundle requires Runyte launcher and macOS 11.0 floor")
    for name in ("runyte", "Runyte"):
        binary = directory / "Contents/MacOS" / name
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError(f"missing bundle executable: {binary}")
        package.check_universal(binary)
        for architecture in ("arm64", "x86_64"):
            build = subprocess.check_output(["vtool", "-arch", architecture, "-show-build", str(binary)], text=True)
            minimums = re.findall(r"^\s*minos\s+(\S+)", build, re.MULTILINE)
            if minimums != ["11.0"] and minimums != ["11.0.0"]:
                raise ValueError(f"{binary} {architecture}: expected minos 11.0, found {minimums}")
            libraries = subprocess.check_output(["otool", "-arch", architecture, "-L", str(binary)], text=True)
            for line in libraries.splitlines()[1:]:
                library = line.strip().split(" (", 1)[0]
                if not library.startswith(("/usr/lib/", "/System/")):
                    raise ValueError(f"{binary} {architecture}: non-system library {library}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--window", action="store_true", help="run preview acceptance on an isolated X11 display")
    args = parser.parse_args()
    directory = args.package.resolve()
    repository = Path(__file__).resolve().parents[2]
    check_layout(directory, repository)
    if (directory / "Contents/MacOS").is_dir():
        check_macos_binaries(directory)
        directory /= "Contents/MacOS"
    binary = directory / "runyte"
    for executable in (binary,):
        if not executable.is_file() or not os.access(executable, os.X_OK):
            parser.error(f"missing packaged executable: {executable}")
    repository = Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory(prefix="runyte-package-check-") as temporary:
        env = {key: value for key, value in os.environ.items() if not key.startswith("RUNYTE_")}
        for variable, name in (("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"),
                               ("XDG_RUNTIME_DIR", "runtime")):
            directory = Path(temporary) / name
            directory.mkdir(mode=0o700)
            env[variable] = str(directory)
        subprocess.run([str(binary), "--version"], env=env, cwd=temporary, check=True, timeout=15)
        # Real engine acceptance uses the packaged helper, not a build-tree copy.
        subprocess.run([sys.executable, str(repository / "crates/runyte-preview/check.py"), "--binary", str(binary)],
                       env=env, cwd=temporary,
                       check=True, timeout=180)
        if args.window:
            # Exercise the packaged executable and its same-build helpers.
            for mode in ([], ["--mux"]):
                subprocess.run([sys.executable, str(repository / "tests/native_window.py"),
                                "--binary", str(binary), "--document-preview", *mode],
                               env=env, cwd=temporary, check=True, timeout=300)


if __name__ == "__main__":
    main()
