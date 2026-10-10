# SPDX-License-Identifier: MPL-2.0
"""Mount an unsigned CI disk image read-only and check its staged contents."""
import argparse
from pathlib import Path
import plistlib
import subprocess


def check(image):
    result = subprocess.check_output(["hdiutil", "attach", "-nobrowse", "-readonly", "-plist", str(image)])
    entities = plistlib.loads(result)["system-entities"]
    device = next(entity["dev-entry"] for entity in entities if "dev-entry" in entity)
    try:
        mount = Path(next(entity["mount-point"] for entity in entities if "mount-point" in entity))
        if not (mount / "Runyte.app/Contents/Info.plist").is_file():
            raise ValueError("disk image omits Runyte.app")
        if not (mount / "README.md").is_file():
            raise ValueError("disk image omits README.md")
        if not (mount / "Applications").is_symlink() or (mount / "Applications").readlink() != Path("/Applications"):
            raise ValueError("disk image requires Applications -> /Applications")
    finally:
        subprocess.run(["hdiutil", "detach", device], check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    check(parser.parse_args().image.resolve(strict=True))
