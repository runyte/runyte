# SPDX-License-Identifier: MPL-2.0
"""Transfer only a same-run, same-platform default debug host, never Cargo caches."""
import argparse
import hashlib
import json
import platform
import shutil
import subprocess
from pathlib import Path


def identity(version):
    return {
        "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "system": platform.system(),
        "machine": platform.machine(),
        "version": version,
        "build": "cargo-test-default-debug-uninstrumented",
    }


def digest(path):
    with path.open("rb") as stream:
        checksum = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            checksum.update(chunk)
        return checksum.hexdigest()


def pack(directory, binary, expected):
    directory.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(binary, directory / "runyte")
    manifest = {**expected, "sha256": digest(directory / "runyte")}
    (directory / "manifest.json").write_text(json.dumps(manifest) + "\n")


def restore(directory, binary, expected):
    manifest = json.loads((directory / "manifest.json").read_text())
    if manifest != {**expected, "sha256": digest(directory / "runyte")}:
        raise ValueError("host artifact source, platform, version or digest mismatch")
    binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(directory / "runyte", binary)
    # upload-artifact does not preserve executable mode.
    binary.chmod(0o755)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("pack", "restore"))
    parser.add_argument("directory", type=Path)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    operation = pack if args.operation == "pack" else restore
    operation(args.directory, args.binary, identity(args.version))


if __name__ == "__main__":
    main()
