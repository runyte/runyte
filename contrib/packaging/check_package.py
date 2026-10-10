# SPDX-License-Identifier: MPL-2.0
"""Exercise compiled desktop executables from an extracted package or app bundle."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--window", action="store_true", help="run preview acceptance on an isolated X11 display")
    args = parser.parse_args()
    directory = args.package.resolve()
    if (directory / "Contents/MacOS").is_dir():
        directory /= "Contents/MacOS"
    binary = directory / "runyte"
    helper = directory / "runyte-preview-helper"
    for executable in (binary, helper):
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
        subprocess.run([sys.executable, str(repository / "contrib/document-preview/check.py")],
                       env={**env, "RUNYTE_PREVIEW_HELPER": str(helper)}, cwd=temporary,
                       check=True, timeout=180)
        if args.window:
            # No helper override: exercises current_exe's sibling discovery.
            subprocess.run([sys.executable, str(repository / "tests/native_window.py"),
                            "--binary", str(binary), "--document-preview", "--packaged-preview-helper"],
                           env=env, cwd=temporary, check=True, timeout=300)


if __name__ == "__main__":
    main()
