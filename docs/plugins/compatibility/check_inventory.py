# SPDX-License-Identifier: MPL-2.0
"""Validate immutable compatibility identities; missing bootstrap pins fail closed."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

DIRECTORY = Path(__file__).resolve().parent
sys.path.insert(0, str(DIRECTORY.parent))
from application import ReleaseRange  # standard-library SDK, no service access
from candidate import write_outputs

SHA = re.compile(r'[0-9a-f]{40}\Z')
DIGEST = re.compile(r'[0-9a-f]{64}\Z')
NAME = re.compile(r'[a-z0-9][a-z0-9-]{0,63}\Z')
REPOSITORY = re.compile(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z')


def keys(value, required, optional=()):
    if not isinstance(value, dict) or set(value) - set(required) - set(optional) or set(required) - set(value):
        raise ValueError('Inventory object has missing or unknown fields: ' + ', '.join(required))


def text(value, pattern, description):
    if not isinstance(value, str) or len(value) > 256 or pattern.fullmatch(value) is None or set(value) == {'0'}:
        raise ValueError('Invalid ' + description)
    return value


def relative(value):
    if not isinstance(value, str) or not value or len(value.encode('utf-8')) > 256 or '\\' in value or any(ord(c) < 32 for c in value):
        raise ValueError('Invalid inventory file path')
    path = PurePosixPath(value)
    if not path.parts or path.is_absolute() or '..' in path.parts or '.' in path.parts or str(path) != value:
        raise ValueError('Inventory file path must be normalized and relative')
    return path


def file_record(value):
    keys(value, ('path', 'sha256'))
    relative(value['path'])
    text(value['sha256'], DIGEST, 'file SHA256')


def unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('Duplicate inventory field: ' + key)
        result[key] = value
    return result


def load_inventory(path=DIRECTORY / 'hosts-and-clients.json'):
    path = Path(path)
    if path.stat().st_size > 256 * 1024:
        raise ValueError('Compatibility inventory exceeds 256 KiB')
    inventory = json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=unique_fields)
    keys(inventory, ('version', 'stable_floor', 'profiles', 'plugins'))
    if type(inventory['version']) is not int or inventory['version'] != 1 or inventory['stable_floor'] != '0.3.0':
        raise ValueError('Unsupported compatibility inventory version or floor')
    for collection in ('profiles', 'plugins'):
        if not isinstance(inventory[collection], list) or not 1 <= len(inventory[collection]) <= 64:
            raise ValueError('Compatibility inventory needs 1–64 pinned ' + collection)
        seen = set()
        for row in inventory[collection]:
            required = ('id', 'repository', 'revision', 'release', 'protocol', 'runyte', 'features', 'files')
            if collection == 'plugins':
                required += ('profile', 'sdk_revision', 'native_tests')
            keys(row, required, ('tag',))
            identifier = text(row['id'], NAME, 'profile/plugin ID')
            if identifier in seen:
                raise ValueError('Duplicate compatibility ID: ' + identifier)
            seen.add(identifier)
            text(row['repository'], REPOSITORY, 'GitHub repository')
            text(row['revision'], SHA, 'immutable source revision')
            if not isinstance(row['release'], str) or not 1 <= len(row['release']) <= 80 or not row['release'].isascii() or any(ord(c) < 32 for c in row['release']):
                raise ValueError('Invalid release/candidate label')
            if 'tag' in row and (not isinstance(row['tag'], str) or re.fullmatch(r'v?[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?', row['tag']) is None):
                raise ValueError('Invalid release tag')
            if row['protocol'] != 'runyte-1':
                raise ValueError('Unsupported protocol profile')
            ReleaseRange(row['runyte'])
            if not isinstance(row['features'], list) or len(row['features']) > 32 or len(set(row['features'])) != len(row['features']) or any(not isinstance(feature, str) or re.fullmatch(r'[A-Za-z0-9_.-]{1,64}', feature) is None for feature in row['features']):
                raise ValueError('Invalid negotiated feature profile')
            expected_files = ('sdk', 'schema', 'fixtures') if collection == 'profiles' else ('sdk', 'schema')
            keys(row['files'], expected_files)
            for value in row['files'].values():
                file_record(value)
            if collection == 'plugins':
                text(row['profile'], NAME, 'plugin client profile')
                text(row['sdk_revision'], SHA, 'vendored SDK source revision')
                tests = row['native_tests']
                if not isinstance(tests, list) or not 1 <= len(tests) <= 64 or len(set(tests)) != len(tests) or any(not isinstance(name, str) or len(name) > 256 or re.fullmatch(r'test_native\.[A-Za-z_][A-Za-z0-9_]*\.test_[A-Za-z0-9_]+', name) is None for name in tests):
                    raise ValueError('Plugin needs explicit native test identities')
    profiles = {row['id']: row for row in inventory['profiles']}
    for row in inventory['plugins']:
        profile = profiles.get(row['profile'])
        if profile is None or row['sdk_revision'] != profile['revision']:
            raise ValueError('Plugin vendor revision must identify its retained client profile')
        for role in ('sdk', 'schema'):
            if row['files'][role]['sha256'] != profile['files'][role]['sha256']:
                raise ValueError('Plugin vendored files differ from its retained client profile')
    return inventory


def verify_files(root, files):
    root = Path(root).resolve()
    for record in files.values():
        path = root.joinpath(*relative(record['path']).parts)
        if path.is_symlink() or not path.resolve().is_relative_to(root) or not path.is_file():
            raise ValueError('Missing or unsafe compatibility artifact: ' + record['path'])
        if path.stat().st_size > 4 * 1024 * 1024:
            raise ValueError('Compatibility artifact exceeds 4 MiB')
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != record['sha256']:
            raise ValueError('Compatibility artifact digest mismatch: ' + record['path'])


def git(checkout, *args):
    return subprocess.run(['git', '-C', str(checkout), *args], check=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True).stdout.strip()


def verify_checkout(checkout, row):
    if git(checkout, 'rev-parse', 'HEAD') != row['revision']:
        raise ValueError('External checkout does not match its immutable revision')
    if git(checkout, 'status', '--porcelain', '--untracked-files=no'):
        raise ValueError('External checkout has modified tracked files')
    if 'tag' in row and git(checkout, 'rev-parse', row['tag'] + '^{commit}') != row['revision']:
        raise ValueError('External release tag does not resolve to its recorded revision')
    verify_files(checkout, row['files'])
    vendor = Path(checkout) / 'VENDOR.md'
    if vendor.is_symlink() or not vendor.is_file() or vendor.stat().st_size > 64 * 1024:
        raise ValueError('Missing or unsafe bounded VENDOR.md provenance')
    provenance = vendor.read_text(encoding='utf-8')
    source = re.search(r'^Source: https://github.com/([^/]+/[^/]+)/tree/([0-9a-f]{40})/', provenance, re.M)
    if source is None or source[1] != 'runyte/runyte' or source[2] != row['sdk_revision']:
        raise ValueError('External VENDOR.md disagrees with its immutable SDK provenance')


def verify_upstream(checkout, profile):
    if profile['repository'] != 'runyte/runyte':
        raise ValueError('Official client profiles need Runyte upstream provenance')
    if 'tag' in profile and git(checkout, 'rev-parse', profile['tag'] + '^{commit}') != profile['revision']:
        raise ValueError('Client source tag disagrees with its recorded revision')
    names = {'sdk': 'application.py', 'schema': 'runyte-1.schema.json', 'fixtures': 'stable-fixtures.json'}
    for role, name in names.items():
        reference = profile['revision'] + ':docs/plugins/' + name
        size = int(git(checkout, 'cat-file', '-s', reference))
        if not 0 < size <= 4 * 1024 * 1024:
            raise ValueError('Upstream compatibility artifact has invalid size')
        source = subprocess.run(['git', '-C', str(checkout), 'show', reference], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout
        if hashlib.sha256(source).hexdigest() != profile['files'][role]['sha256']:
            raise ValueError('Retained artifact differs from its recorded upstream source: ' + role)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', type=Path, default=DIRECTORY / 'hosts-and-clients.json')
    parser.add_argument('--plugin')
    parser.add_argument('--verify-upstream', action='store_true')
    parser.add_argument('--checkout', type=Path)
    parser.add_argument('--github-output', type=Path)
    args = parser.parse_args()
    inventory = load_inventory(args.inventory)
    for profile in inventory['profiles']:
        verify_files(args.inventory.parent, profile['files'])
        if args.verify_upstream:
            verify_upstream(DIRECTORY.parents[2], profile)
    values = {'plugins': json.dumps([{'id': row['id'], 'repository': row['repository'], 'revision': row['revision']} for row in inventory['plugins']], separators=(',', ':'))}
    if args.plugin:
        rows = [row for row in inventory['plugins'] if row['id'] == args.plugin]
        if len(rows) != 1:
            raise ValueError('Requested plugin is not in the pinned inventory')
        row = rows[0]
        if args.checkout:
            verify_checkout(args.checkout, row)
        values.update(repository=row['repository'], revision=row['revision'], plugin=row['id'])
    elif args.checkout:
        parser.error('--checkout requires --plugin')
    if args.github_output:
        write_outputs(values, args.github_output)
    print(json.dumps(values, sort_keys=True))


if __name__ == '__main__':
    main()
