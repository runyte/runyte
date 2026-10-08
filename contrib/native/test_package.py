# SPDX-License-Identifier: MPL-2.0
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess
import tempfile
import unittest

import package


class NativePackageTests(unittest.TestCase):
    def test_linux_desktop_registration_uses_exact_identity_and_quoted_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / 'Runyte % " $ ` \\ editor'
            shutil.copy2(package.HERE.parents[1] / "src/fixtures/stand-in", binary)
            desktop = package.install_linux(binary, root / "data", refresh=False)
            contents = desktop.read_text()
            self.assertEqual(desktop.name, package.APP_ID + ".desktop")
            self.assertIn("Icon=" + package.APP_ID + "\n", contents)
            self.assertIn("StartupWMClass=" + package.APP_ID + "\n", contents)
            self.assertIn(" --window --editor %F\n", contents)
            self.assertIn("%%", contents)
            for size in (16, 32, 64, 128, 256, 512, 1024):
                image = root / f"data/icons/hicolor/{size}x{size}/apps/{package.APP_ID}.png"
                self.assertEqual(struct.unpack(">II", image.read_bytes()[16:24]), (size, size))
            if validator := shutil.which("desktop-file-validate"):
                subprocess.run([validator, str(desktop)], check=True)

    def test_macos_bundle_contains_icon_launch_flags_and_notices(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            # The fixture is copied as data and never executed.
            binary = package.HERE.parents[1] / "src/fixtures/stand-in"
            destination = package.bundle_macos(binary, root / "Runyte.app")
            resources = destination / "Contents/Resources"
            with (destination / "Contents/Info.plist").open("rb") as source:
                info = plistlib.load(source)
            self.assertEqual(info["CFBundleIdentifier"], package.APP_ID)
            self.assertEqual(info["CFBundleIconFile"], "Runyte.icns")
            launcher = destination / "Contents/MacOS" / info["CFBundleExecutable"]
            self.assertTrue(os.access(launcher, os.X_OK))
            self.assertIn('--window --editor "$@"', launcher.read_text())
            self.assertIn("/opt/homebrew/bin:/usr/local/bin", launcher.read_text())
            for notice in ("LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md", "licenses/jetbrains-mono/OFL.txt",
                           "licenses/jetbrains-mono/NERD-FONTS-LICENSE.txt", "assets/fonts/jetbrains-mono/README.md"):
                self.assertTrue((resources / notice).is_file(), notice)
            icon = (resources / "Runyte.icns").read_bytes()
            self.assertEqual(icon[:4], b"icns")
            self.assertEqual(struct.unpack(">I", icon[4:8])[0], len(icon))
            self.assertIn(b"ic10", icon)
            with self.assertRaises(ValueError):
                package.bundle_macos(binary, destination)
            self.assertEqual((resources / "Runyte.icns").read_bytes(), icon)

    def test_desktop_path_cannot_inject_additional_keys(self):
        for value in ("/tmp/editor\nTerminal=true", "/tmp/editor\r", "/tmp/editor\0"):
            with self.assertRaises(ValueError):
                package.desktop_argument(value)


if __name__ == "__main__":
    unittest.main()
