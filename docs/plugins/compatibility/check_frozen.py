# SPDX-License-Identifier: MPL-2.0
"""Exercise immutable clients and an external plugin against a candidate host.

Schemas are a development dependency. The external plugin's runtime and tests
remain standard-library code, and each run owns temporary data/configuration.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

from check_inventory import DIRECTORY, load_inventory, verify_checkout, verify_files


def read_fixtures(path):
    if path.stat().st_size > 4 * 1024 * 1024:
        raise ValueError('Compatibility fixtures exceed 4 MiB')
    fixtures = json.loads(path.read_text(encoding='utf-8'))
    if not isinstance(fixtures, list) or not fixtures:
        raise ValueError('Compatibility fixtures must not be empty')
    for fixture in fixtures:
        if (not isinstance(fixture, dict) or fixture.get('direction') not in ('host', 'plugin')
                or not isinstance(fixture.get('message'), dict)
                or not isinstance(fixture.get('features', []), list)
                or any(not isinstance(name, str) for name in fixture.get('features', []))):
            raise ValueError('Invalid directional compatibility fixture')
    return fixtures


def validator(schema, direction):
    from jsonschema import Draft202012Validator
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator({**schema, 'anyOf': [{'$ref': '#/$defs/' + direction + 'Message'}]})


def frozen_handshake(profile, sdk_path, fixtures):
    name = '_runyte_frozen_' + profile['id'].replace('-', '_')
    spec = importlib.util.spec_from_file_location(name, sdk_path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    register = next(f['message'] for f in fixtures if f['direction'] == 'plugin' and f['message'].get('type') == 'register')
    host = [f['message'] for f in fixtures if f['direction'] == 'host']
    hello = next(value for value in host if value['type'] == 'hello')
    acknowledged = next(value for value in host if value['type'] == 'registered')
    app = module.Application(register['name'], register['commands'], register['required_capabilities'],
                             runyte=register['runyte'], optional_capabilities=register['optional_capabilities'],
                             required_features=register['required_features'], optional_features=register['optional_features'])
    # Ignorable host properties and unknown advertisements must remain harmless
    # to a base-only pinned decoder. No new operation is sent without selection.
    hello = {**hello, 'features': [*hello['features'], 'compatibility.future'], 'compatibility_future': True}
    acknowledged = {**acknowledged, 'compatibility_future': True}
    messages = iter([hello, acknowledged])
    def read():
        try:
            return next(messages)
        except StopIteration:
            raise EOFError from None
    sent = []
    app._read = read
    app._write = sent.append
    app.run()
    if len(sent) != 1 or sent[0]['type'] != 'register' or sent[0]['version'] != profile['protocol']:
        raise ValueError('Frozen SDK failed its base-profile handshake')
    return sent[0]


def check_profiles(inventory, root):
    current_schema = json.loads((DIRECTORY.parent / 'runyte-1.schema.json').read_text(encoding='utf-8'))
    current_fixtures = read_fixtures(DIRECTORY.parent / 'stable-fixtures.json')
    current_plugin = validator(current_schema, 'plugin')
    for profile in inventory['profiles']:
        verify_files(root, profile['files'])
        paths = {role: root / record['path'] for role, record in profile['files'].items()}
        frozen_schema = json.loads(paths['schema'].read_text(encoding='utf-8'))
        frozen_fixtures = read_fixtures(paths['fixtures'])
        old_host = validator(frozen_schema, 'host')
        old_plugin_count = new_host_count = 0
        for fixture in frozen_fixtures:
            if fixture['direction'] == 'plugin':
                current_plugin.validate(fixture['message'])
                old_plugin_count += 1
        for fixture in current_fixtures:
            if fixture['direction'] == 'host' and set(fixture.get('features', [])).issubset(profile['features']):
                old_host.validate(fixture['message'])
                new_host_count += 1
        if old_plugin_count == 0 or new_host_count == 0:
            raise ValueError('Frozen profile did not exercise both traffic directions')
        current_plugin.validate(frozen_handshake(profile, paths['sdk'], frozen_fixtures))
        print(f"Frozen {profile['id']} ({profile['revision']}): {old_plugin_count} old plugin frames, {new_host_count} current host frames, SDK handshake", flush=True)


def host_version(binary):
    result = subprocess.run([str(binary), '--version'], check=True, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, timeout=10)
    match = re.fullmatch(r'runyte ([^\s]+)\s*', result.stdout)
    if match is None or len(result.stdout) > 256:
        raise ValueError('Candidate executable did not identify its Runyte version')
    return match[1]


def test_cases(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from test_cases(test)
        else:
            yield test


def require_native_tests(suite, names):
    discovered = {case.id() for case in test_cases(suite)}
    if not set(names).issubset(discovered):
        raise ValueError('Required native tests are missing: ' + ', '.join(sorted(set(names) - discovered)))


def check_test_result(result, required):
    skipped = {case.id() for case, _reason in result.skipped}
    if not result.wasSuccessful() or result.testsRun == 0 or skipped.intersection(required) or any(name.startswith('test_native.') for name in skipped):
        raise ValueError('External compatibility suite failed or skipped a required native test')


def run_external(checkout, binary, row, expected_version):
    from application import ReleaseRange
    checkout, binary = checkout.resolve(), binary.resolve()
    verify_checkout(checkout, row)
    actual = host_version(binary)
    if actual != expected_version:
        raise ValueError('Built host version disagrees with the candidate gate')
    if not ReleaseRange(row['runyte']).contains(actual):
        raise ValueError('Frozen acceptance lane excludes host ' + actual + '; a breaking-release migration needs an explicit rejection scenario and a new accepted plugin profile')
    print(f"External {row['id']}={row['revision']} SDK={row['sdk_revision']} host={actual} expected=accept", flush=True)
    variables = ('RUNYTE_BIN', 'PYTHONDONTWRITEBYTECODE', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME',
                 'XDG_CACHE_HOME', 'XDG_STATE_HOME', 'XDG_RUNTIME_DIR')
    previous = {name: os.environ.get(name) for name in variables}
    previous_directory = Path.cwd()
    previous_path = sys.path[:]
    try:
        with tempfile.TemporaryDirectory(prefix='runyte-frozen-') as temporary:
            root = Path(temporary)
            os.environ['RUNYTE_BIN'] = str(binary)
            os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
            for name in variables[2:]:
                directory = root / name.lower()
                directory.mkdir(mode=0o700)
                os.environ[name] = str(directory)
            os.chdir(checkout)
            sys.path.insert(0, str(checkout))
            suite = unittest.TestLoader().discover(str(checkout / 'tests'))
            require_native_tests(suite, row['native_tests'])
            result = unittest.TextTestRunner(verbosity=2).run(suite)
            check_test_result(result, row['native_tests'])
    finally:
        os.chdir(previous_directory)
        sys.path[:] = previous_path
        for name, value in previous.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', type=Path, default=DIRECTORY / 'hosts-and-clients.json')
    parser.add_argument('--checkout', type=Path)
    parser.add_argument('--plugin', default='ru-time-v1')
    parser.add_argument('--host-bin', type=Path)
    parser.add_argument('--host-version')
    args = parser.parse_args()
    inventory = load_inventory(args.inventory)
    check_profiles(inventory, args.inventory.resolve().parent)
    if any(value is not None for value in (args.checkout, args.host_bin, args.host_version)):
        if any(value is None for value in (args.checkout, args.host_bin, args.host_version)):
            parser.error('External checks require --checkout, --host-bin and --host-version together')
        rows = [row for row in inventory['plugins'] if row['id'] == args.plugin]
        if len(rows) != 1:
            parser.error('Requested plugin is absent from the inventory')
        run_external(args.checkout, args.host_bin, rows[0], args.host_version)


if __name__ == '__main__':
    main()
