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
DESKTOP_LIBRARIES = "".join(
    f"\t{name} (libc6,x86-64) => /usr/lib/x86_64-linux-gnu/{name}\n"
    for name in ("libxcb.so.1", "libxkbcommon.so.0", "libxkbcommon-x11.so.0"))
REQUIREMENT = ('-R=anchor apple generic and identifier "com.runyte.Runyte" '
               'and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
               'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists '
               'and certificate leaf[subject.OU] = "DP96UQZ969"')
INFO_PLIST = b"""<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.runyte.Runyte</string>
</dict></plist>
"""


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
                     "sha256sum", "shasum", "ln", "readlink"):
            path = shutil.which(tool)
            if path:
                (self.bin / tool).symlink_to(path)
        for tool in ("curl", "uname", "getconf"):
            self.mock(tool, 'exec "$INSTALL_TEST_PYTHON" "$INSTALL_TEST_DRIVER" --tool '
                      + shlex.quote(tool) + ' "$@"\n')
        # Never consult the host's loader cache, signature tools or version.
        (self.root / "ldconfig-cache").write_text(DESKTOP_LIBRARIES)
        self.mock("ldconfig", 'while IFS= read -r line; do printf "%s\\n" "$line"; done'
                  ' < "$INSTALL_TEST_ROOT/ldconfig-cache"\n')
        self.mock("sw_vers", "printf '%s\\n' 15.1\n")
        self.mock("codesign", 'printf \'%s\\n\' "$*" >> "$INSTALL_TEST_ROOT/codesign"\n')
        self.mock("spctl", 'printf \'%s\\n\' "$*" >> "$INSTALL_TEST_ROOT/spctl"\n')
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

    def make_archive(self, target="x86_64-unknown-linux-gnu", member=None, payload=PAYLOAD, prefix="runyte-terminal"):
        name = f"{prefix}-v{VERSION}-{target}.tar.xz"
        path = self.assets / name
        with tarfile.open(path, "w:xz") as archive:
            entry = tarfile.TarInfo(member or f"{prefix}-{VERSION}-{target}/runyte")
            entry.size = len(payload)
            archive.addfile(entry, io.BytesIO(payload))
        self.write_checksum(path)
        return path

    def make_desktop_archive(self, target, files, payload=PAYLOAD):
        """A desktop archive whose package directory holds `files` plus runyte."""
        package = f"runyte-desktop-{VERSION}-{target}"
        path = self.assets / f"runyte-desktop-v{VERSION}-{target}.tar.xz"
        executable = "Runyte.app/Contents/MacOS/runyte" if "apple-darwin" in target else "runyte"
        files = {executable: payload, **files}
        with tarfile.open(path, "w:xz") as archive:
            directories = {package}
            for name in files:
                parts = name.split("/")[:-1]
                directories.update(package + "/" + "/".join(parts[:i]) for i in range(1, len(parts) + 1))
            for directory in sorted(directories):
                entry = tarfile.TarInfo(directory)
                entry.type, entry.mode = tarfile.DIRTYPE, 0o755
                archive.addfile(entry)
            for name, data in files.items():
                entry = tarfile.TarInfo(f"{package}/{name}")
                entry.size, entry.mode = len(data), 0o755
                archive.addfile(entry, io.BytesIO(data))
        self.write_checksum(path)
        return path

    def macos_desktop(self, payload=PAYLOAD):
        self.settings.update(os="Darwin", arch="arm64")
        return self.make_desktop_archive("universal-apple-darwin",
                                         {"Runyte.app/Contents/Info.plist": INFO_PLIST}, payload)

    def requested(self):
        return (self.root / "requests").read_text().splitlines()

    def write_checksum(self, path):
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        (self.assets / "SHA256SUMS").write_text(f"{digest}  {path.name}\n")

    def install(self, *args, success=True, shell="/bin/sh", piped=False, kept_previous=False):
        # Written archive payloads are inspected only, never executed by the
        # installed-version probe. Executable probes use the checked-in inode.
        if self.destination.is_file() and not self.destination.is_symlink():
            if self.destination.read_bytes() != (ROOT / "src/fixtures/stand-in").read_bytes():
                self.destination.chmod(0o644)
        (self.root / "settings.json").write_text(json.dumps(self.settings))
        command = [shell, "-s", "--"] if piped else [shell, str(ROOT / "install.sh")]
        result = subprocess.run(command + list(args), env=self.env, text=True,
                                input=(ROOT / "install.sh").read_text() if piped else None,
                                capture_output=True, timeout=20)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertEqual(list(self.scratch.iterdir()), [], "temporary download leaked")
        self.assertEqual(list(self.home.rglob(".runyte-install.*")), [], "staging file leaked")
        if success and not kept_previous:
            self.assertEqual(list(self.home.rglob(".runyte-previous.*")), [], "previous app leaked")
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
                                  RELEASES + f"/download/v{VERSION}/SHA256SUMS",
                                  RELEASES + f"/download/v{VERSION}/" + self.archive.name])
        # An open reader of the old inode must remain undisturbed by replacement.
        with self.destination.open("rb") as old:
            self.make_archive(payload=b"new editor")
            self.install(piped=True)
            self.assertEqual(old.read(), PAYLOAD)
        self.assertEqual(self.destination.read_bytes(), b"new editor")

    def test_historical_layout_is_selected_from_manifest(self):
        archive = self.make_archive(prefix="runyte")
        self.install()
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)
        self.assertTrue((self.root / "requests").read_text().endswith(archive.name + "\n"))

    def test_modern_layout_wins_when_both_are_listed(self):
        modern = (self.assets / "SHA256SUMS").read_text()
        self.make_archive(prefix="runyte", payload=b"historical")
        sums = self.assets / "SHA256SUMS"
        sums.write_text(sums.read_text() + modern)
        self.install()
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)

    def test_manifest_without_either_layout_fails_before_archive_download(self):
        (self.assets / "SHA256SUMS").write_text("0" * 64 + "  unrelated.tar.xz\n")
        result = self.install(success=False)
        self.assertIn("no terminal archive", result.stderr)
        self.assertEqual(len((self.root / "requests").read_text().splitlines()), 2)

    def installed_desktop_edition(self):
        # A hard link keeps the executable inode unwritten and must stay on
        # the repository filesystem; the installer deliberately refuses symlinks.
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        fixture = tempfile.TemporaryDirectory(prefix="installer-desktop-", dir=target)
        self.addCleanup(fixture.cleanup)
        self.destination = Path(fixture.name) / "runyte"
        os.link(ROOT / "src/fixtures/stand-in", self.destination)
        self.destination.with_name("runyte.behavior").write_text(
            "printf '%s\\n' 'runyte 0.4.0 (desktop edition)'\n")

    def test_an_update_keeps_the_installed_desktop_edition(self):
        self.installed_desktop_edition()
        self.settings["libc"] = "glibc 2.39"
        desktop = self.make_desktop_archive("x86_64-unknown-linux-gnu", {}, b"desktop editor")
        self.install("--install-dir", str(self.destination.parent))
        self.assertEqual(self.requested()[-1], RELEASES + f"/download/v{VERSION}/" + desktop.name)
        self.assertEqual(self.destination.read_bytes(), b"desktop editor")

    def test_terminal_option_replaces_an_installed_desktop_edition(self):
        self.installed_desktop_edition()
        result = self.install("--terminal", "--install-dir", str(self.destination.parent))
        self.assertEqual(self.requested()[-1], RELEASES + f"/download/v{VERSION}/" + self.archive.name)
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)
        self.assertIn("Replaced the desktop edition with the terminal edition", result.stdout)

    def test_linux_desktop_installs_the_desktop_archive_beside_runed(self):
        self.settings["libc"] = "glibc 2.39"
        self.existing_installation()
        desktop = self.make_desktop_archive("x86_64-unknown-linux-gnu", {}, b"desktop editor")
        result = self.install("--desktop")
        self.assertEqual(self.requested()[-1], RELEASES + f"/download/v{VERSION}/" + desktop.name)
        self.assertEqual(self.destination.read_bytes(), b"desktop editor")
        self.assertEqual(os.readlink(self.destination.with_name("runed")), "runyte")
        self.assertIn("Python 3 is required to register a desktop launcher", result.stdout)
        self.assertFalse((self.root / "codesign").exists())

    def test_linux_desktop_registers_its_launcher_with_the_packaged_script(self):
        self.settings["libc"] = "glibc 2.39"
        self.make_desktop_archive("aarch64-unknown-linux-gnu",
                                  {"contrib/packaging/package.py": b"# registration fixture\n"})
        self.settings["arch"] = "aarch64"
        self.mock("python3", 'printf \'%s\\n\' "$@" > "$INSTALL_TEST_ROOT/python3-args"\n')
        self.assertIn("Registered the Runyte desktop launcher", self.install("--desktop").stdout)
        script, *arguments = (self.root / "python3-args").read_text().splitlines()
        self.assertTrue(script.endswith(f"/runyte-desktop-{VERSION}-aarch64-unknown-linux-gnu/contrib/packaging/package.py"))
        self.assertEqual(arguments, ["linux", "--binary", str(self.destination)])

    def test_linux_desktop_library_check_uses_ld_library_path_and_tolerates_ldconfig_failure(self):
        self.settings["libc"] = "glibc 2.39"
        self.make_desktop_archive("x86_64-unknown-linux-gnu", {})
        (self.root / "ldconfig-cache").write_text(DESKTOP_LIBRARIES.splitlines()[0] + "\n")
        libraries = self.root / "nix libraries"
        libraries.mkdir()
        for name in ("libxkbcommon.so.0", "libxkbcommon-x11.so.0"):
            (libraries / name).write_bytes(b"")
        self.env["LD_LIBRARY_PATH"] = f"/absent:{libraries}"
        self.install("--desktop")
        del self.env["LD_LIBRARY_PATH"]
        self.mock("ldconfig", "exit 1\n")
        self.install("--desktop")

    def test_an_installation_whose_edition_cannot_be_read_needs_an_option(self):
        self.installed_desktop_edition()
        self.destination.with_name("runyte.behavior").write_text("exit 1\n")
        result = self.install("--install-dir", str(self.destination.parent), success=False)
        self.assertIn("rerun with --desktop or --terminal", result.stderr)
        self.assertFalse((self.root / "requests").exists())
        self.install("--terminal", "--install-dir", str(self.destination.parent))
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)

    def test_linux_desktop_reports_why_registration_failed(self):
        self.settings["libc"] = "glibc 2.39"
        self.make_desktop_archive("x86_64-unknown-linux-gnu",
                                  {"contrib/packaging/package.py": b"# registration fixture\n"})
        self.mock("python3", "printf '%s\\n' 'Traceback' 'OSError: read-only data directory' >&2\nexit 1\n")
        result = self.install("--desktop")
        self.assertIn("Could not register a desktop launcher (OSError: read-only data directory)", result.stdout)
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)

    def test_linux_desktop_prerequisites_fail_before_download(self):
        self.make_desktop_archive("x86_64-unknown-linux-gnu", {})
        for libc, cache, message in (
            ("glibc 2.38", DESKTOP_LIBRARIES, "requires glibc 2.39 or newer (found glibc 2.38)"),
            ("glibc 2.39", DESKTOP_LIBRARIES.splitlines()[0] + "\n",
             "needs libxkbcommon.so.0 libxkbcommon-x11.so.0;"),
        ):
            with self.subTest(libc=libc):
                self.settings["libc"] = libc
                (self.root / "ldconfig-cache").write_text(cache)
                self.assertIn(message, self.install("--desktop", success=False).stderr)
                self.assertFalse((self.root / "requests").exists())
                self.assertFalse(self.destination.exists())
        # The terminal edition keeps its own floor and needs no window libraries.
        self.settings["libc"] = "glibc 2.35"
        self.make_archive()
        self.install("--terminal")

    def test_a_release_without_the_desktop_edition_fails_before_archive_download(self):
        self.settings["libc"] = "glibc 2.39"
        result = self.install("--desktop", success=False)
        self.assertIn(f"Runyte {VERSION} has no desktop edition for x86_64-unknown-linux-gnu", result.stderr)
        self.assertEqual(len(self.requested()), 2)

    def test_macos_desktop_installs_the_app_and_links_runyte_to_it(self):
        desktop = self.macos_desktop()
        self.existing_installation()
        result = self.install("--desktop")
        app = self.home / "Applications/Runyte.app"
        self.assertEqual(self.requested()[-1], RELEASES + f"/download/v{VERSION}/" + desktop.name)
        self.assertEqual(os.readlink(self.destination), str(app / "Contents/MacOS/runyte"))
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)
        self.assertEqual(os.readlink(self.destination.with_name("runed")), "runyte")
        self.assertEqual((app / "Contents/Info.plist").read_bytes(), INFO_PLIST)
        verified, = (self.root / "codesign").read_text().splitlines()
        self.assertTrue(verified.startswith("--verify --deep --strict " + REQUIREMENT + " "), verified)
        self.assertTrue(verified.endswith("/Runyte.app"), verified)
        assessed, = (self.root / "spctl").read_text().splitlines()
        self.assertTrue(assessed.startswith("--assess --type execute "), assessed)
        self.assertTrue(assessed.endswith("/Runyte.app"), assessed)
        self.assertIn("Installed Runyte.app", result.stdout)

    def test_macos_desktop_update_keeps_the_app_directory_and_replaces_the_app(self):
        self.macos_desktop()
        applications = self.home / "My Apps"
        self.install("--desktop", "--app-dir", str(applications))
        app = applications / "Runyte.app"
        (app / "Contents/stale").write_bytes(b"from the previous version")
        with (app / "Contents/MacOS/runyte").open("rb") as running:
            self.macos_desktop(b"new desktop editor")
            self.install()
            self.assertEqual(running.read(), PAYLOAD)
        self.assertEqual(os.readlink(self.destination), str(app / "Contents/MacOS/runyte"))
        self.assertEqual(self.destination.read_bytes(), b"new desktop editor")
        self.assertFalse((app / "Contents/stale").exists())
        self.assertFalse((self.home / "Applications").exists())

    def test_macos_desktop_refusals_preserve_the_installed_app(self):
        self.macos_desktop()
        self.install("--desktop")
        app = self.home / "Applications/Runyte.app"
        link = os.readlink(self.destination)
        self.macos_desktop(b"unverified editor")
        self.mock("codesign", "exit 1\n")
        self.assertIn("not signed by the Runyte release team", self.install(success=False).stderr)
        self.mock("codesign", "exit 0\n")
        self.mock("spctl", "exit 3\n")
        self.assertIn("Gatekeeper rejected", self.install(success=False).stderr)
        self.mock("spctl", "exit 0\n")
        (app / "Contents/Info.plist").write_text("<string>org.example.Other</string>")
        (self.root / "requests").unlink()
        self.assertIn("not Runyte", self.install(success=False).stderr)
        self.assertFalse((self.root / "requests").exists())
        self.assertEqual((app / "Contents/MacOS/runyte").read_bytes(), PAYLOAD)
        self.assertEqual(os.readlink(self.destination), link)
        self.mock("sw_vers", "printf '%s\\n' 10.15.7\n")
        self.assertIn("requires macOS 11 or newer", self.install(success=False).stderr)

    def mock_failing_app_moves(self, *sources):
        """mv that refuses to move an app out of the named hidden directories."""
        real = shutil.which("mv", path=os.defpath + ":/usr/bin:/bin")
        cases = "|".join(f"*/{source}.*/Runyte.app" for source in sources)
        self.mock("mv", f'case "$1" in {cases}) exit 1 ;; esac\nexec {shlex.quote(real)} "$@"\n')

    def test_a_failed_app_replacement_restores_the_previous_app(self):
        self.macos_desktop()
        self.install("--desktop")
        app = self.home / "Applications/Runyte.app"
        self.macos_desktop(b"new desktop editor")
        self.mock_failing_app_moves(".runyte-install")
        self.assertIn(f"Could not replace {app}", self.install(success=False).stderr)
        self.assertEqual((app / "Contents/MacOS/runyte").read_bytes(), PAYLOAD)
        self.assertEqual(list(app.parent.glob(".runyte-previous.*")), [])
        # When even the restore fails, the previous app is kept, never deleted.
        self.mock_failing_app_moves(".runyte-install", ".runyte-previous")
        result = self.install(success=False)
        kept, = app.parent.glob(".runyte-previous.*")
        self.assertIn(f"the previous app is in {kept}", result.stderr)
        self.assertEqual((kept / "Runyte.app/Contents/MacOS/runyte").read_bytes(), PAYLOAD)

    def test_an_interrupted_app_replacement_restores_the_previous_app(self):
        self.macos_desktop()
        self.install("--desktop")
        app = self.home / "Applications/Runyte.app"
        self.macos_desktop(b"new desktop editor")
        # TERM reaches the installer while the previous app is set aside.
        real = shutil.which("mv", path=os.defpath + ":/usr/bin:/bin")
        self.mock("mv", 'case "$1" in */.runyte-install.*/Runyte.app) kill -TERM "$PPID"; exit 1 ;; esac\n'
                        f'exec {shlex.quote(real)} "$@"\n')
        result = self.install(success=False)
        # The trap restored it: the failure path's own message never ran.
        self.assertNotIn("Could not replace", result.stderr)
        self.assertEqual((app / "Contents/MacOS/runyte").read_bytes(), PAYLOAD)
        self.assertEqual(list(app.parent.glob(".runyte-*")), [])

    def test_a_previous_app_that_cannot_be_removed_does_not_stop_the_link(self):
        self.macos_desktop()
        first = self.home / "First Apps"
        self.install("--desktop", "--app-dir", str(first))
        locked = first / "Runyte.app/Contents/Locked"
        locked.mkdir()
        (locked / "file").write_bytes(b"kept")
        locked.chmod(0o555)
        self.macos_desktop(b"new desktop editor")
        result = self.install(kept_previous=True)
        kept, = first.glob(".runyte-previous.*")
        locked = kept / "Runyte.app/Contents/Locked"
        self.addCleanup(locked.chmod, 0o755)
        self.assertIn(f"Could not remove the previous app in {kept}", result.stdout)
        self.assertEqual(self.destination.read_bytes(), b"new desktop editor")
        self.assertIn("Linked", result.stdout)

    def test_an_unwritable_application_directory_fails_before_download(self):
        self.macos_desktop()
        applications = self.home / "Shared Apps"
        applications.mkdir()
        applications.chmod(0o555)
        self.addCleanup(applications.chmod, 0o755)
        result = self.install("--desktop", "--app-dir", str(applications), success=False)
        self.assertIn(f"Cannot write to {applications}", result.stderr)
        self.assertFalse((self.root / "requests").exists())

    def test_macos_desktop_app_destination_problems_fail_before_download(self):
        self.macos_desktop()
        applications = self.home / "Applications"
        for app_dir, prepare, message in (
            (applications / "Runyte.app", None, "inside an application bundle"),
            (applications, lambda app: app.symlink_to(self.home), "Not an application bundle"),
            (applications, lambda app: app.write_bytes(b"file"), "Not an application bundle"),
        ):
            with self.subTest(message=message, app_dir=app_dir):
                applications.mkdir(exist_ok=True)
                if prepare:
                    prepare(applications / "Runyte.app")
                result = self.install("--desktop", "--app-dir", str(app_dir), success=False)
                self.assertIn(message, result.stderr)
                self.assertFalse((self.root / "requests").exists())
                (applications / "Runyte.app").unlink(missing_ok=True)

    def test_macos_desktop_archive_and_tool_problems_preserve_the_installation(self):
        self.macos_desktop()
        self.existing_installation()
        self.make_desktop_archive("universal-apple-darwin", {}, b"")
        archive = self.assets / f"runyte-desktop-v{VERSION}-universal-apple-darwin.tar.xz"
        with tarfile.open(archive, "w:xz") as broken:
            entry = tarfile.TarInfo(f"runyte-desktop-{VERSION}-universal-apple-darwin/Runyte.app/Contents/Info.plist")
            entry.size = len(INFO_PLIST)
            broken.addfile(entry, io.BytesIO(INFO_PLIST))
        self.write_checksum(archive)
        self.assertIn("no Runyte.app executable", self.install("--desktop", success=False).stderr)
        for tool in ("spctl", "codesign"):
            (self.bin / tool).unlink()
            self.assertIn(f"Required tool is missing: {tool}", self.install("--desktop", success=False).stderr)
        self.assertEqual(self.destination.read_bytes(), b"old editor")
        self.assertFalse((self.home / "Applications/Runyte.app").exists())

    def test_macos_desktop_link_variants(self):
        self.macos_desktop()
        first = self.home / "First Apps"
        second = self.home / "Second Apps"
        self.install("--desktop", "--app-dir", str(first))
        # Another directory installs a second app and says the first remains.
        result = self.install("--desktop", "--app-dir", str(second))
        self.assertIn(f"previously linked app {first / 'Runyte.app'} was left in place", result.stdout)
        self.assertEqual(os.readlink(self.destination), str(second / "Runyte.app/Contents/MacOS/runyte"))
        # The same directory spelled with a trailing slash is not another app.
        result = self.install("--desktop", "--app-dir", str(second) + "//")
        self.assertNotIn("left in place", result.stdout)
        self.assertEqual(os.readlink(self.destination), str(second / "Runyte.app/Contents/MacOS/runyte"))
        # A link whose app was deleted is reinstalled where it pointed.
        shutil.rmtree(second)
        self.install()
        self.assertEqual(self.destination.read_bytes(), PAYLOAD)
        # A relative link is not one this installer made.
        self.destination.unlink()
        self.destination.symlink_to("../../First Apps/Runyte.app/Contents/MacOS/runyte")
        self.assertIn("Refusing to replace a symlink", self.install(success=False).stderr)

    def test_disk_image_links_are_taken_over_and_runed_follows_runyte(self):
        self.macos_desktop()
        self.install("--desktop", "--app-dir", str(self.home / "Disk Image Apps"))
        app = self.home / "Disk Image Apps/Runyte.app/Contents/MacOS/runyte"
        runed = self.destination.with_name("runed")
        # The links INSTALLATION.md gives for an app copied from the disk image.
        for name in (self.destination, runed):
            name.unlink()
            name.symlink_to(app)
        self.make_archive("aarch64-apple-darwin", payload=b"terminal editor")
        result = self.install("--terminal")
        self.assertEqual(os.readlink(runed), "runyte")
        self.assertEqual(runed.read_bytes(), b"terminal editor")
        self.assertNotIn("unchanged", result.stdout)

    def test_terminal_option_replaces_the_desktop_link_and_keeps_the_app(self):
        self.macos_desktop()
        self.install("--desktop")
        self.make_archive("aarch64-apple-darwin", payload=b"terminal editor")
        result = self.install("--terminal")
        self.assertFalse(self.destination.is_symlink())
        self.assertEqual(self.destination.read_bytes(), b"terminal editor")
        self.assertTrue((self.home / "Applications/Runyte.app/Contents/MacOS/runyte").exists())
        self.assertIn("was left in place", result.stdout)

    def test_runed_is_a_relative_link_to_runyte(self):
        self.install()
        runed = self.destination.with_name("runed")
        self.assertTrue(runed.is_symlink())
        self.assertEqual(os.readlink(runed), "runyte")
        self.assertEqual(runed.read_bytes(), PAYLOAD)
        # An update keeps the link, which follows the replaced executable.
        self.make_archive(payload=b"new editor")
        self.install()
        self.assertEqual(os.readlink(runed), "runyte")
        self.assertEqual(runed.read_bytes(), b"new editor")

    def test_an_existing_runed_that_is_not_the_link_is_left_alone(self):
        self.destination.parent.mkdir(parents=True, exist_ok=True)
        runed = self.destination.with_name("runed")
        runed.write_bytes(b"someone else's runed")
        self.assertIn("unchanged", self.install().stdout)
        self.assertEqual(runed.read_bytes(), b"someone else's runed")
        runed.unlink()
        runed.symlink_to("elsewhere")
        self.assertIn("unchanged", self.install().stdout)
        self.assertEqual(os.readlink(runed), "elsewhere")

    def test_pinned_version_custom_directory_and_bash(self):
        destination = self.home / "custom bin"
        for version in (VERSION, "v" + VERSION):
            # Like install(), never let the edition probe run written bytes.
            if (destination / "runyte").exists():
                (destination / "runyte").chmod(0o644)
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
                     ("--install-dir", "relative"), ("--desktop", "--terminal"),
                     ("--app-dir",), ("--app-dir", "relative"),
                     ("--app-dir", "/absolute"), ("--version", "1.2"),
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
