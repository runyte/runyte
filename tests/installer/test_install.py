# SPDX-License-Identifier: MPL-2.0
"""Offline installer acceptance using real archives, hashes, tar and file moves.

Run with: python3 -m unittest discover -s tests/installer -v
Network and platform probes use the repository's checked-in stand-in executable.
The fake editor bytes are inspected after installation, never executed.
"""

import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
RELEASES = "https://github.com/runyte/runyte/releases"
VERSION = "0.3.7"
PAYLOAD = b"fixture editor bytes, never executed\n"


def tool_main(tool, args):
    """Invoked by stand-in; all state is local to one temporary fixture."""
    root = Path(os.environ["INSTALL_TEST_ROOT"])
    settings = json.loads((root / "settings.json").read_text())
    if tool == "uname":
        print(settings["os"] if args == ["-s"] else settings["arch"])
    elif tool == "getconf":
        if settings["libc"] == "musl":
            return 1
        print(settings["libc"])
    elif tool == "curl":
        assert args[0] == "-q", args
        for flag in ("--fail", "--silent", "--show-error", "--location"):
            assert flag in args, args
        for flag in ("--proto", "--proto-redir"):
            assert args[args.index(flag) + 1] == "=https", args
        url = args[-1]
        with (root / "requests").open("a") as log:
            log.write(url + "\n")
        if settings.get("fail_download") == url.rsplit("/", 1)[-1]:
            return 22
        if url == RELEASES + "/latest":
            print(settings.get("latest", RELEASES + "/tag/v" + VERSION), end="")
        else:
            # Fail closed: a wrong version or filename cannot use any fixture.
            assert url.startswith(RELEASES + "/download/v" + VERSION + "/"), url
            source = root / "assets" / url.rsplit("/", 1)[-1]
            shutil.copyfile(source, args[args.index("--output") + 1])
    else:
        raise AssertionError(tool)
    return 0


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="runyte-installer-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "tools"
        self.bin.mkdir()
        self.assets = self.root / "assets"
        self.assets.mkdir()
        self.scratch = self.root / "tmp"
        self.scratch.mkdir()
        self.home = self.root / "home with spaces"
        self.home.mkdir()
        self.destination = self.home / ".local/bin/runyte"
        self.settings = {"os": "Linux", "arch": "x86_64", "libc": "glibc 2.35"}
        for tool in ("awk", "tar", "xz", "mktemp", "mkdir", "chmod", "mv", "rm",
                     "sha256sum", "shasum"):
            path = shutil.which(tool)
            if path:
                (self.bin / tool).symlink_to(path)
        for tool in ("curl", "uname", "getconf"):
            self.mock(tool, 'exec "$INSTALL_TEST_PYTHON" "$INSTALL_TEST_DRIVER" --tool '
                      + shlex.quote(tool) + ' "$@"\n')
        self.env = dict(os.environ, HOME=str(self.home), TMPDIR=str(self.scratch),
                        PATH=str(self.bin), XDG_CONFIG_HOME=str(self.root / "config"),
                        INSTALL_TEST_ROOT=str(self.root), INSTALL_TEST_PYTHON=sys.executable,
                        INSTALL_TEST_DRIVER=str(Path(__file__).resolve()))
        self.archive = self.make_archive()

    def mock(self, tool, behavior):
        path = self.bin / tool
        path.unlink(missing_ok=True)
        path.symlink_to(ROOT / "src/fixtures/stand-in")
        path.with_name(tool + ".behavior").write_text(behavior)

    def make_archive(self, target="x86_64-unknown-linux-gnu", member=None, payload=PAYLOAD):
        name = f"runyte-v{VERSION}-{target}.tar.xz"
        path = self.assets / name
        with tarfile.open(path, "w:xz") as archive:
            entry = tarfile.TarInfo(member or f"runyte-{VERSION}-{target}/runyte")
            entry.size = len(payload)
            archive.addfile(entry, io.BytesIO(payload))
        self.write_checksum(path)
        return path

    def write_checksum(self, path):
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        (self.assets / "SHA256SUMS").write_text(f"{digest}  {path.name}\n")

    def install(self, *args, success=True, shell="/bin/sh", piped=False):
        (self.root / "settings.json").write_text(json.dumps(self.settings))
        command = [shell, "-s", "--"] if piped else [shell, str(ROOT / "install.sh")]
        result = subprocess.run(command + list(args), env=self.env, text=True,
                                input=(ROOT / "install.sh").read_text() if piped else None,
                                capture_output=True, timeout=20)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertEqual(list(self.scratch.iterdir()), [], "temporary download leaked")
        self.assertEqual(list(self.home.rglob(".runyte-install.*")), [], "staging file leaked")
        return result

    def existing_installation(self):
        self.destination.parent.mkdir(parents=True, exist_ok=True)
        self.destination.write_bytes(b"old editor")

    def test_latest_install_and_update_use_same_command(self):
        self.install(piped=True)
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)
        self.assertEqual(self.destination.stat().st_mode & 0o777, 0o755)
        requests = (self.root / "requests").read_text().splitlines()
        self.assertEqual(requests, [RELEASES + "/latest",
                                  RELEASES + f"/download/v{VERSION}/" + self.archive.name,
                                  RELEASES + f"/download/v{VERSION}/SHA256SUMS"])
        # An open reader of the old inode must remain undisturbed by replacement.
        with self.destination.open("rb") as old:
            self.make_archive(payload=b"new editor")
            self.install(piped=True)
            self.assertEqual(old.read(), PAYLOAD)
        self.assertEqual(self.destination.read_bytes(), b"new editor")

    def test_pinned_version_custom_directory_and_bash(self):
        destination = self.home / "custom bin"
        for version in (VERSION, "v" + VERSION):
            self.install("--version", version, "--install-dir", str(destination), shell="/bin/bash")
        self.assertEqual((destination / "runyte").read_bytes(), PAYLOAD)
        self.assertNotIn("/latest", (self.root / "requests").read_text())

    def test_all_supported_platform_mappings(self):
        for os_name, arch, target in (
            ("Linux", "x86_64", "x86_64-unknown-linux-gnu"),
            ("Linux", "aarch64", "aarch64-unknown-linux-gnu"),
            ("Darwin", "x86_64", "x86_64-apple-darwin"),
            ("Darwin", "arm64", "aarch64-apple-darwin"),
        ):
            with self.subTest(os=os_name, arch=arch):
                self.settings.update(os=os_name, arch=arch)
                self.make_archive(target)
                self.install()
                self.assertEqual(self.destination.read_bytes(), PAYLOAD)

    def test_shasum_fallback(self):
        (self.bin / "sha256sum").unlink(missing_ok=True)
        self.assertTrue((self.bin / "shasum").exists(), "shasum needed for fallback acceptance")
        self.install()

    def test_unsupported_platforms_and_libc_fail_before_download(self):
        for override in ({"os": "FreeBSD"}, {"arch": "armv7l"},
                         {"libc": "musl"}, {"libc": "glibc 2.34"}):
            with self.subTest(override=override):
                original = self.settings.copy()
                self.settings.update(override)
                self.install(success=False)
                self.assertFalse((self.root / "requests").exists())
                self.settings = original

    def test_bad_arguments_and_versions(self):
        for args in (("--unknown",), ("--version",), ("--install-dir",),
                     ("--version", ""), ("--install-dir", ""),
                     ("--install-dir", "relative"), ("--version", "1.2"),
                     ("--version", "1.2.3/elsewhere"), ("--version", "01.2.3"),
                     ("--version", "1.2.3\n4.5.6")):
            with self.subTest(args=args):
                self.install(*args, success=False)
                self.assertFalse((self.root / "requests").exists())

    def test_help_needs_no_tools(self):
        self.env["PATH"] = str(self.root / "absent")
        self.assertIn("update", self.install("--help").stdout)

    def test_unset_home_requires_an_explicit_directory(self):
        del self.env["HOME"]
        self.assertIn("HOME is unset", self.install(success=False).stderr)
        destination = self.home / "explicit bin"
        self.install("--install-dir", str(destination))
        self.assertEqual((destination / "runyte").read_bytes(), PAYLOAD)

    def test_unusable_install_directory(self):
        path = self.home / "file"
        path.write_bytes(b"keep")
        self.install("--install-dir", str(path / "bin"), success=False)
        self.assertEqual(path.read_bytes(), b"keep")

    def test_missing_dependencies(self):
        for tool in ("curl", "tar", "sha256sum", "shasum"):
            (self.bin / tool).unlink(missing_ok=True)
        self.assertIn("Required tool", self.install(success=False).stderr)

    def test_missing_checksum_tools(self):
        for tool in ("sha256sum", "shasum"):
            (self.bin / tool).unlink(missing_ok=True)
        self.assertIn("sha256sum or shasum", self.install(success=False).stderr)

    def test_unexpected_latest_redirect(self):
        for url in ("https://example.com/tag/v1.2.3", RELEASES + "/tag/v1.2.3-rc1"):
            with self.subTest(url=url):
                self.settings["latest"] = url
                self.install(success=False)
                self.assertFalse(self.destination.exists())

    def test_failed_downloads_preserve_existing_installation(self):
        self.existing_installation()
        for name in ("latest", self.archive.name, "SHA256SUMS"):
            with self.subTest(name=name):
                self.settings["fail_download"] = name
                self.assertIn("Could not", self.install(success=False).stderr)
                self.assertEqual(self.destination.read_bytes(), b"old editor")

    def test_checksum_failures_preserve_existing_installation(self):
        self.existing_installation()
        sums = self.assets / "SHA256SUMS"
        valid = sums.read_text()
        for manifest in ("", valid + valid, valid.replace(self.archive.name, "other.tar.xz"),
                         "0" * 64 + "  " + self.archive.name + "\n",
                         "z" * 64 + "  " + self.archive.name + "\n",
                         "abc  " + self.archive.name + "\n", valid.rstrip() + " extra\n"):
            with self.subTest(manifest=manifest):
                sums.write_text(manifest)
                self.install(success=False)
                self.assertEqual(self.destination.read_bytes(), b"old editor")

    def test_corrupt_download_is_rejected(self):
        self.archive.write_bytes(b"corrupt download")
        self.assertIn("SHA-256 mismatch", self.install(success=False).stderr)
        self.assertFalse(self.destination.exists())

    def test_bad_archives_preserve_existing_installation(self):
        self.existing_installation()
        self.archive.write_bytes(b"not an archive")
        self.write_checksum(self.archive)
        self.assertIn("extract", self.install(success=False).stderr)
        for member, payload in (("unexpected/runyte", PAYLOAD), (None, b""),
                                ("../../escaped", PAYLOAD)):
            self.make_archive(member=member, payload=payload)
            self.install(success=False)
        self.assertEqual(self.destination.read_bytes(), b"old editor")
        self.assertFalse((self.home / ".local/escaped").exists())

    def test_symlink_and_directory_destinations_are_refused(self):
        self.destination.parent.mkdir(parents=True)
        other = self.home / "other editor"
        other.write_bytes(b"other")
        self.destination.symlink_to(other)
        self.assertIn("symlink", self.install(success=False).stderr)
        self.assertEqual(other.read_bytes(), b"other")
        self.destination.unlink()
        self.destination.mkdir()
        self.assertIn("Not a regular file", self.install(success=False).stderr)
        self.assertEqual(list(self.destination.iterdir()), [])

    def test_failed_final_operations_preserve_existing_installation(self):
        self.existing_installation()
        for tool in ("chmod", "mv"):
            with self.subTest(tool=tool):
                self.mock(tool, "exit 1\n")
                self.install(success=False)
                self.assertEqual(self.destination.read_bytes(), b"old editor")
                (self.bin / tool).unlink()
                (self.bin / tool).symlink_to(shutil.which(tool))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--tool":
        sys.exit(tool_main(sys.argv[2], sys.argv[3:]))
    unittest.main()
