# SPDX-License-Identifier: MPL-2.0
"""Prepare a real versioned stable candidate without changing the checkout.

Only the pre-stable bootstrap stages source. At and after 0.3.0, gates use the
actual checked-out package version and source. No runtime version override exists.
"""
import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess

FLOOR = '0.3.0'
EXCLUDED = {'.git', '.runyte', 'target', '__pycache__', '.pytest_cache', '.venv', 'venv'}
VERSION = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?\Z')


def package_version(manifest):
    section = re.search(r'^\[package\]\s*\n(.*?)(?=^\[|\Z)', manifest, re.M | re.S)
    if section is None:
        raise ValueError('Cargo.toml has no package section')
    matches = list(re.finditer(r'^version\s*=\s*"([^"\n]+)"[^\n]*$', section[1], re.M))
    if len(matches) != 1 or VERSION.fullmatch(matches[0][1]) is None:
        raise ValueError('Cargo.toml needs one explicit semantic package version')
    return matches[0][1]


def replace_package_version(manifest, version):
    old = package_version(manifest)
    section = re.search(r'^\[package\]\s*\n(.*?)(?=^\[|\Z)', manifest, re.M | re.S)
    replacement = re.sub(r'^(version\s*=\s*")' + re.escape(old) + r'(")',
                         lambda match: match[1] + version + match[2], section[1], count=1, flags=re.M)
    return manifest[:section.start(1)] + replacement + manifest[section.end(1):]


def replace_lock_version(lock, old, new):
    blocks = list(re.finditer(r'^\[\[package\]\]\n(.*?)(?=^\[\[package\]\]|\Z)', lock, re.M | re.S))
    blocks = [block for block in blocks if re.search(r'^name = "runyte"$', block[1], re.M)]
    if len(blocks) != 1:
        raise ValueError('Cargo.lock needs exactly one root runyte package')
    block = blocks[0]
    replacement, count = re.subn(r'^version = "' + re.escape(old) + '"$',
                                 'version = "' + new + '"', block[1], count=1, flags=re.M)
    if count != 1:
        raise ValueError('Cargo.lock root version disagrees with Cargo.toml')
    return lock[:block.start(1)] + replacement + lock[block.end(1):]


def intended_files(source, run=subprocess.run):
    result = run(['git', '-C', str(source), 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
                 check=True, stdout=subprocess.PIPE)
    for raw in sorted(set(result.stdout.split(b'\0')) - {b''}):
        path = PurePosixPath(os.fsdecode(raw))
        if path.is_absolute() or '..' in path.parts:
            raise ValueError('Source inventory contains an unsafe path')
        if any(part in EXCLUDED for part in path.parts):
            continue
        original = source.joinpath(*path.parts)
        # Deleted tracked files and nested checkouts are not source to copy.
        if not original.exists() and not original.is_symlink():
            continue
        if original.is_symlink():
            target = os.readlink(original)
            if os.path.isabs(target) or not original.resolve().is_relative_to(source):
                raise ValueError('Candidate symlink escapes the staged source: ' + str(path))
        elif not original.is_file():
            continue
        yield original, path


def prepare(source, destination, run=subprocess.run):
    source = Path(source).resolve()
    manifest = (source / 'Cargo.toml').read_text(encoding='utf-8')
    original_version = package_version(manifest)
    core = tuple(map(int, VERSION.fullmatch(original_version).groups()))
    mode = 'exact'
    if core < (0, 3, 0):
        destination = Path(destination).resolve()
        if destination == source or destination.is_relative_to(source):
            raise ValueError('Candidate destination must be outside the source checkout')
        files = list(intended_files(source, run))
        destination.mkdir(parents=True, exist_ok=False)
        try:
            for original, relative in files:
                target = destination.joinpath(*relative.parts)
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(original, target, follow_symlinks=False)
            lock = (destination / 'Cargo.lock').read_text(encoding='utf-8')
            expected_lock = replace_lock_version(lock, original_version, FLOOR)
            (destination / 'Cargo.toml').write_text(replace_package_version(manifest, FLOOR), encoding='utf-8')
            # Cargo owns lockfile updates. Only the candidate package version may
            # differ; dependency drift fails this bootstrap rather than hiding it.
            run(['cargo', 'check', '--lib'], cwd=destination, check=True)
            if (destination / 'Cargo.lock').read_text(encoding='utf-8') != expected_lock:
                raise ValueError('Candidate cargo check changed dependencies or failed to update the root version')
        except BaseException:
            shutil.rmtree(destination)
            raise
        source = destination
        mode = 'bootstrap-candidate'
    version = package_version((source / 'Cargo.toml').read_text(encoding='utf-8'))
    target = Path(os.environ.get('CARGO_TARGET_DIR', 'target'))
    target = (target if target.is_absolute() else source / target).resolve()
    return {'source': str(source), 'host_version': version, 'mode': mode, 'target_dir': str(target)}


def write_outputs(values, path):
    if any('\n' in str(value) or '\r' in str(value) for value in values.values()):
        raise ValueError('Workflow outputs cannot contain line breaks')
    with Path(path).open('a', encoding='utf-8') as output:
        for key, value in values.items():
            output.write(f'{key}={value}\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument('--destination', required=True, type=Path)
    parser.add_argument('--github-output', type=Path)
    args = parser.parse_args()
    result = prepare(args.source, args.destination)
    if args.github_output:
        write_outputs(result, args.github_output)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
