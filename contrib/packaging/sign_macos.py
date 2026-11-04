# SPDX-License-Identifier: MPL-2.0
"""Sign and notarize macOS distribution artifacts using an existing Keychain profile.

`--keychain` limits the identity and profile lookups to one keychain file, such
as the temporary keychain a release runner creates from its encrypted secrets.
"""
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


def keychain_options(keychain):
    return ["--keychain", keychain] if keychain is not None else []


def notarize(commands, artifact, profile, keychain=None):
    credentials = ["--keychain-profile", profile, *keychain_options(keychain)]
    # A capped wait fails a stuck submission with Apple's last status rather
    # than holding the release job until its own time limit.
    response = commands.run("xcrun", "notarytool", "submit", artifact, *credentials,
                            "--wait", "--timeout", "30m", "--output-format", "json",
                            capture=True, check=False)
    if commands.dry_run:
        print("Require status Accepted; on rejection, fetch: " + shlex.join([
            "xcrun", "notarytool", "log", "<submission-id>", *map(str, credentials)]))
        return
    try:
        result = json.loads(response.stdout)
        if not isinstance(result, dict):
            raise ValueError("notarytool response is not an object")
    except (ValueError, TypeError) as error:
        diagnostic = response.stderr.strip() or response.stdout.strip() or "no diagnostic output"
        raise RuntimeError(f"notarytool failed (exit {response.returncode}): {diagnostic}") from error
    status = result.get("status")
    if status != "Accepted":
        # Apple keeps a log only for a finished submission. One still in
        # progress when the wait ran out has none yet, and asking would replace
        # this error with notarytool's.
        if result.get("id") and status in ("Invalid", "Rejected"):
            commands.run("xcrun", "notarytool", "log", result["id"], *credentials, check=False)
        submission = f" (submission {result['id']})" if result.get("id") else ""
        raise RuntimeError(f"notarization was not accepted: {status or 'missing status'}{submission}")
    response.check_returncode()


def sign_app(commands, app, identity, profile, keychain=None):
    for artifact in (app / "Contents/MacOS/runyte", app / "Contents/MacOS/RunyteLauncher", app):
        commands.run("codesign", "--force", "--options", "runtime", "--timestamp",
                     *keychain_options(keychain), "--sign", identity, artifact)
    def submit(archive):
        commands.run("ditto", "-c", "-k", "--keepParent", app, archive)
        notarize(commands, archive, profile, keychain)
    if commands.dry_run:
        submit(Path("<temporary>") / "Runyte.zip")
    else:
        with tempfile.TemporaryDirectory(prefix="runyte-notary-") as temporary:
            submit(Path(temporary) / "Runyte.zip")
    commands.run("xcrun", "stapler", "staple", app)


def sign_dmg(commands, dmg, identity, profile, keychain=None):
    commands.run("codesign", "--force", "--timestamp", *keychain_options(keychain), "--sign", identity, dmg)
    notarize(commands, dmg, profile, keychain)
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
            child.add_argument("--keychain", type=Path,
                               help="keychain file holding the identity and profile (default: search list)")
            child.add_argument("artifact", type=Path)
        else:
            child.add_argument("app", type=Path)
            child.add_argument("dmg", type=Path)
    args = parser.parse_args()
    commands = Commands(args.dry_run)
    if args.command == "app":
        sign_app(commands, args.artifact.resolve(), args.identity, args.notary_profile, args.keychain)
    elif args.command == "dmg":
        sign_dmg(commands, args.artifact.resolve(), args.identity, args.notary_profile, args.keychain)
    else:
        verify(commands, args.app.resolve(), args.dmg.resolve())


if __name__ == "__main__":
    main()
