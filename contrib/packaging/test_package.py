# SPDX-License-Identifier: MPL-2.0
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import package
import check_package


class NativePackageTests(unittest.TestCase):
    def test_linux_package_has_one_editor_and_relocatable_registration(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = package.HERE.parents[1] / "src/fixtures/stand-in"
            build = root / "build"
            build.mkdir()
            # Packaging fixtures are inspected as data, never executed.
            shutil.copy2(fixture, build / "runyte")
            destination = package.bundle_linux(build / "runyte", root / "package")
            moved = root / "moved package"
            destination.rename(moved)
            self.assertEqual((moved / "runyte").read_bytes(), fixture.read_bytes())
            self.assertFalse((moved / "runyte-preview-helper").exists())
            self.assertEqual((moved / "runed").resolve(), moved / "runyte")
            for name in ("licenses", "docs/user-guide.md", "THIRD_PARTY_NOTICES.md",
                         "contrib/packaging/package.py", "contrib/packaging/icons/Runyte.icns"):
                self.assertTrue((moved / name).exists(), name)
            desktop = package.install_linux(moved / "runyte", root / "data", refresh=False)
            self.assertIn(package.desktop_argument(str(moved / "runyte")), desktop.read_text())
            with self.assertRaises(ValueError):
                package.bundle_linux(build / "runyte", moved)

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

    @patch("package.subprocess.check_output", return_value="arm64 x86_64\n")
    def test_macos_bundle_contains_icon_launch_flags_and_notices(self, _output):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            # The fixture is copied as data and never executed.
            binary = package.HERE.parents[1] / "src/fixtures/stand-in"
            destination = package.bundle_macos(binary, root / "Runyte.app", binary)
            self.assertEqual({p.name for p in (destination / "Contents/MacOS").iterdir()}, {"runyte", "Runyte", "runed"})
            resources = destination / "Contents/Resources"
            with (destination / "Contents/Info.plist").open("rb") as source:
                info = plistlib.load(source)
            self.assertEqual(info["CFBundleIdentifier"], package.APP_ID)
            self.assertEqual(info["CFBundleIconFile"], "Runyte.icns")
            launcher = destination / "Contents/MacOS" / info["CFBundleExecutable"]
            self.assertTrue(os.access(launcher, os.X_OK))
            self.assertEqual(launcher.read_bytes(), binary.read_bytes())
            self.assertEqual(info["CFBundleExecutable"], "Runyte")
            self.assertEqual(info["LSMinimumSystemVersion"], "11.0")
            self.assertEqual(info["LSApplicationCategoryType"], "public.app-category.developer-tools")
            for notice in ("LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md", "licenses/jetbrains-mono/OFL.txt",
                           "licenses/jetbrains-mono/NERD-FONTS-LICENSE.txt", "assets/fonts/jetbrains-mono/README.md"):
                self.assertTrue((resources / notice).is_file(), notice)
            icon = (resources / "Runyte.icns").read_bytes()
            self.assertEqual(icon[:4], b"icns")
            self.assertEqual(struct.unpack(">I", icon[4:8])[0], len(icon))
            self.assertIn(b"ic10", icon)
            with self.assertRaises(ValueError):
                package.bundle_macos(binary, destination, binary)
            self.assertEqual((resources / "Runyte.icns").read_bytes(), icon)

    @patch("package.subprocess.check_output", return_value="arm64 x86_64\n")
    def test_package_layout_rejects_extra_files_and_wrong_links(self, _output):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repository = package.HERE.parents[1]
            binary = repository / 'src/fixtures/stand-in'
            for platform, bundle in [('linux', package.bundle_linux), ('macos', lambda binary, destination: package.bundle_macos(binary, destination, binary))]:
                destination = bundle(binary, root / platform)
                check_package.check_layout(destination, repository)
                extra = destination / 'unexpected-helper'
                extra.write_text('not an executable')
                with self.assertRaisesRegex(ValueError, 'extra='):
                    check_package.check_layout(destination, repository)
                extra.unlink()
            link = root / 'linux/runed'
            link.unlink()
            link.symlink_to('missing')
            with self.assertRaisesRegex(ValueError, 'runed'):
                check_package.check_layout(root / 'linux', repository)

    def test_macos_binary_checks_reject_wrong_floor_and_homebrew_libraries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = package.HERE.parents[1] / "src/fixtures/stand-in"
            with patch("package.subprocess.check_output", return_value="arm64 x86_64\n"):
                bundle = package.bundle_macos(binary, root / "Runyte.app", binary)
            for minimum, library, accepted in [
                ("11.0", "/usr/lib/libSystem.B.dylib", True),
                ("11.0", "/System/Library/Frameworks/AppKit.framework/AppKit", True),
                ("14.0", "/usr/lib/libSystem.B.dylib", False),
                ("11.0", "/opt/homebrew/lib/libfontconfig.dylib", False),
                ("11.0", "@rpath/libcustom.dylib", False),
            ]:
                def output(command, **kwargs):
                    return {"lipo": "arm64 x86_64\n", "vtool": f" minos {minimum}\n",
                            "otool": f"editor:\n\t{library} (compatibility version 1.0.0)\n"}[command[0]]
                with patch("package.subprocess.check_output", side_effect=output):
                    if accepted:
                        check_package.check_macos_binaries(bundle)
                    else:
                        with self.assertRaises(ValueError):
                            check_package.check_macos_binaries(bundle)
            with patch("package.subprocess.check_output", return_value="arm64\n"):
                with self.assertRaisesRegex(ValueError, "arm64 and x86_64"):
                    package.bundle_macos(binary, root / "incomplete.app", binary)
                self.assertFalse((root / "incomplete.app").exists())

    def test_desktop_path_cannot_inject_additional_keys(self):
        for value in ("/tmp/editor\nTerminal=true", "/tmp/editor\r", "/tmp/editor\0"):
            with self.assertRaises(ValueError):
                package.desktop_argument(value)


if __name__ == "__main__":
    unittest.main()
