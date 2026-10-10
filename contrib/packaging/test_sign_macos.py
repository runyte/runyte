# SPDX-License-Identifier: MPL-2.0
import contextlib
import io
import json
import plistlib
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import package
import check_dmg
import sign_macos


class DistributionToolsTests(unittest.TestCase):
    def test_mounted_image_is_detached_even_when_contents_are_invalid(self):
        with tempfile.TemporaryDirectory() as temporary:
            mount = Path(temporary)
            (mount / "Runyte.app/Contents").mkdir(parents=True)
            (mount / "Runyte.app/Contents/Info.plist").write_bytes(b"fixture")
            (mount / "README.md").write_text("fixture")
            (mount / "Applications").symlink_to("/Applications")
            response = plistlib.dumps({"system-entities": [{"dev-entry": "/dev/disk-test"}, {"mount-point": str(mount)}]})
            with patch("check_dmg.subprocess.check_output", return_value=response) as attach, patch("check_dmg.subprocess.run") as detach:
                check_dmg.check(Path("image.dmg"))
                self.assertIn("-readonly", attach.call_args.args[0])
                detach.assert_called_with(["hdiutil", "detach", "/dev/disk-test"], check=True)
                (mount / "README.md").unlink()
                with self.assertRaises(ValueError):
                    check_dmg.check(Path("image.dmg"))
                self.assertEqual(detach.call_count, 2)

    def test_notary_cli_failure_preserves_diagnostic_without_stapling(self):
        for stdout in ("", "not json", "[]"):
            with patch("sign_macos.subprocess.run", return_value=subprocess.CompletedProcess([], 1, stdout, "No Keychain profile named test-profile")) as run, contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(RuntimeError, "No Keychain profile"):
                    sign_macos.notarize(sign_macos.Commands(), Path("image.dmg"), "test-profile")
                self.assertEqual(run.call_count, 1)

    def test_disk_image_stages_relocatable_app_and_refuses_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = package.HERE.parents[1] / "src/fixtures/stand-in"
            with patch("package.subprocess.check_output", return_value="arm64 x86_64\n"):
                app = package.bundle_macos(fixture, root / "Runyte.app", fixture)
            version = plistlib.loads((app / "Contents/Info.plist").read_bytes())["CFBundleShortVersionString"]
            output = root / "Runyte.dmg"
            stages = []
            def create(command, **kwargs):
                self.assertEqual(command[:4], ["hdiutil", "create", "-volname", f"Runyte {version}"])
                self.assertNotIn("-ov", command)
                self.assertEqual(command[-3:], ["-format", "UDZO", str(output)])
                stage = Path(command[command.index("-srcfolder") + 1])
                stages.append(stage)
                self.assertEqual((stage / "Applications").readlink(), Path("/Applications"))
                self.assertEqual((stage / "Runyte.app/Contents/MacOS/runed").readlink(), Path("runyte"))
                self.assertTrue((stage / "README.md").is_file())
                output.write_bytes(b"mock disk image")
            with patch("package.subprocess.run", side_effect=create) as run:
                package.disk_image(app, output)
                with self.assertRaisesRegex(ValueError, "already exists"):
                    package.disk_image(app, output)
                self.assertEqual(run.call_count, 1)
            self.assertFalse(stages[0].exists())

    def test_dry_run_prints_inside_out_signing_without_execution(self):
        output = io.StringIO()
        with patch("sign_macos.subprocess.run") as run, contextlib.redirect_stdout(output):
            commands = sign_macos.Commands(dry_run=True)
            sign_macos.sign_app(commands, Path("Runyte.app"), "Developer ID Application: Example", "notary-test")
            sign_macos.sign_dmg(commands, Path("Runyte.dmg"), "Developer ID Application: Example", "notary-test")
            sign_macos.verify(commands, Path("Runyte.app"), Path("Runyte.dmg"))
            run.assert_not_called()
        lines = output.getvalue().splitlines()
        signing = [line for line in lines if line.startswith("codesign --force")]
        self.assertEqual(len(signing), 4)
        self.assertTrue(signing[0].endswith("Runyte.app/Contents/MacOS/runyte"))
        self.assertTrue(signing[1].endswith("Runyte.app/Contents/MacOS/Runyte"))
        self.assertTrue(signing[2].endswith("Runyte.app"))
        self.assertTrue(all("--options runtime" in line for line in signing[:3]))
        self.assertNotIn("--options runtime", signing[3])
        self.assertTrue(all("--deep" not in line for line in signing))
        self.assertIn("codesign --verify --deep --strict --verbose=2 Runyte.app", lines)
        self.assertIn("spctl --assess --type open --context context:primary-signature -vv Runyte.dmg", lines)
        self.assertIn("xcrun stapler validate Runyte.app", lines)
        self.assertIn("xcrun stapler validate Runyte.dmg", lines)

    def test_notarization_status_gates_stapling_and_fetches_failure_log(self):
        for status, code in [("Accepted", 0), ("Invalid", 0), ("Rejected", 1), ("In Progress", 0), (None, 0)]:
            with self.subTest(status=status), contextlib.redirect_stdout(io.StringIO()):
                calls = []
                def run(command, **kwargs):
                    calls.append(command)
                    response = json.dumps({"status": status, "id": "submission-test"})
                    return subprocess.CompletedProcess(command, code if "submit" in command else 0, response, "")
                with patch("sign_macos.subprocess.run", side_effect=run):
                    commands = sign_macos.Commands()
                    if status == "Accepted":
                        sign_macos.sign_dmg(commands, Path("Runyte.dmg"), "test-identity", "test-profile")
                        self.assertEqual(calls[-1], ["xcrun", "stapler", "staple", "Runyte.dmg"])
                    else:
                        with self.assertRaises(RuntimeError):
                            sign_macos.sign_dmg(commands, Path("Runyte.dmg"), "test-identity", "test-profile")
                        self.assertEqual(calls[-1], ["xcrun", "notarytool", "log", "submission-test", "--keychain-profile", "test-profile"])
                        self.assertFalse(any("staple" in call for call in calls))


if __name__ == "__main__":
    unittest.main()
