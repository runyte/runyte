# SPDX-License-Identifier: MPL-2.0
"""Sign and notarize macOS distribution artifacts using an existing Keychain profile."""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import tempfile


class Commands:
    def __init__(self, dry_run=False):
        self.dry_run = dry_run

    def run(self, *arguments, capture=False, check=True):
        command = [str(argument) for argument in arguments]
        print(shlex.join(command), flush=True)
        if self.dry_run:
            return None
        result = subprocess.run(command, check=check, text=True, capture_output=capture)
        return result if capture else None


def notarize(commands, artifact, profile):
    response = commands.run("xcrun", "notarytool", "submit", artifact,
                            "--keychain-profile", profile, "--wait", "--output-format", "json",
                            capture=True, check=False)
    if commands.dry_run:
        print("Require status Accepted; on rejection, fetch: " + shlex.join([
            "xcrun", "notarytool", "log", "<submission-id>", "--keychain-profile", profile]))
        return
    try:
        result = json.loads(response.stdout)
        if not isinstance(result, dict):
            raise ValueError("notarytool response is not an object")
    except (ValueError, TypeError) as error:
        diagnostic = response.stderr.strip() or response.stdout.strip() or "no diagnostic output"
        raise RuntimeError(f"notarytool failed (exit {response.returncode}): {diagnostic}") from error
    if result.get("status") != "Accepted":
        if result.get("id"):
            commands.run("xcrun", "notarytool", "log", result["id"], "--keychain-profile", profile)
        raise RuntimeError(f"notarization was not accepted: {result.get('status', 'missing status')}")
    response.check_returncode()


def sign_app(commands, app, identity, profile):
    for artifact in (app / "Contents/MacOS/runyte", app / "Contents/MacOS/RunyteLauncher", app):
        commands.run("codesign", "--force", "--options", "runtime", "--timestamp", "--sign", identity, artifact)
    def submit(archive):
        commands.run("ditto", "-c", "-k", "--keepParent", app, archive)
        notarize(commands, archive, profile)
    if commands.dry_run:
        submit(Path("<temporary>") / "Runyte.zip")
    else:
        with tempfile.TemporaryDirectory(prefix="runyte-notary-") as temporary:
            submit(Path(temporary) / "Runyte.zip")
    commands.run("xcrun", "stapler", "staple", app)


def sign_dmg(commands, dmg, identity, profile):
    commands.run("codesign", "--force", "--timestamp", "--sign", identity, dmg)
    notarize(commands, dmg, profile)
    commands.run("xcrun", "stapler", "staple", dmg)


def verify(commands, app, dmg):
    commands.run("codesign", "--verify", "--deep", "--strict", "--verbose=2", app)
    commands.run("spctl", "--assess", "--type", "execute", "-vv", app)
    commands.run("spctl", "--assess", "--type", "open", "--context", "context:primary-signature", "-vv", dmg)
    for artifact in (app, dmg):
        commands.run("xcrun", "stapler", "validate", artifact)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    for name in ("app", "dmg", "verify"):
        child = subparsers.add_parser(name)
        child.add_argument("--dry-run", action="store_true", help="print commands without running them")
        if name != "verify":
            child.add_argument("--identity", required=True, help="Developer ID Application certificate identity")
            child.add_argument("--notary-profile", required=True, help="existing Keychain notarytool profile")
            child.add_argument("artifact", type=Path)
        else:
            child.add_argument("app", type=Path)
            child.add_argument("dmg", type=Path)
    args = parser.parse_args()
    commands = Commands(args.dry_run)
    if args.command == "app":
        sign_app(commands, args.artifact.resolve(), args.identity, args.notary_profile)
    elif args.command == "dmg":
        sign_dmg(commands, args.artifact.resolve(), args.identity, args.notary_profile)
    else:
        verify(commands, args.app.resolve(), args.dmg.resolve())


if __name__ == "__main__":
    main()
