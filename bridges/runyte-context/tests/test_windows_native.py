# SPDX-License-Identifier: MPL-2.0
"""Private Windows Bridge/MCP acceptance against the real Rust named-pipe host."""

import json
import os
from pathlib import Path
import subprocess
import sys
import time
import unittest


def discovery_fixture(arguments):
    mode = arguments[0]
    if mode == 'success':
        os.write(1, b'{"bounded":true}')
    elif mode == 'oversized':
        os.write(1, b'x' * int(arguments[1]))
    elif mode == 'timeout':
        time.sleep(float(arguments[1]))
    elif mode == 'hold-stdout':
        subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), '--discovery-fixture',
             'timeout', arguments[1]],
            stdin=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=False,
        )
    else:
        raise AssertionError(f'unknown discovery fixture mode: {mode}')


if __name__ == '__main__' and sys.argv[1:2] == ['--discovery-fixture']:
    discovery_fixture(sys.argv[2:])
    raise SystemExit(0)


from runyte_context.client import Bridge
from runyte_context.server import PROTOCOL, Server


FIXTURE_KEYS = (
    'RUNYTE_CONTEXT_NATIVE_RECORD',
    'RUNYTE_CONTEXT_NATIVE_ROOT',
    'RUNYTE_CONTEXT_NATIVE_PREAUTH_READY',
    'RUNYTE_CONTEXT_NATIVE_PREAUTH_CONTINUE',
    'RUNYTE_CONTEXT_NATIVE_READY',
    'RUNYTE_CONTEXT_NATIVE_CONTINUE',
)
NATIVE_FIXTURE = sys.platform == 'win32' and all(os.environ.get(key) for key in FIXTURE_KEYS)
PUBLIC_KEYS = (
    'RUNYTE_CONTEXT_PUBLIC_BINARY',
    'RUNYTE_CONTEXT_PUBLIC_ROOT',
    'RUNYTE_CONTEXT_PUBLIC_PHASE_READY',
    'RUNYTE_CONTEXT_PUBLIC_PHASE_CONTINUE',
    'RUNYTE_CONTEXT_PUBLIC_RESTART_READY',
    'RUNYTE_CONTEXT_PUBLIC_REVOKE_CONTINUE',
)
PUBLIC_FIXTURE = sys.platform == 'win32' and all(os.environ.get(key) for key in PUBLIC_KEYS)


def started(bridge):
    server = Server(bridge)
    initialized = server.handle({
        'jsonrpc': '2.0',
        'id': 1,
        'method': 'initialize',
        'params': {
            'protocolVersion': PROTOCOL,
            'capabilities': {},
            'clientInfo': {'name': 'native-acceptance', 'version': '1'},
        },
    })
    if initialized[0].get('result', {}).get('protocolVersion') != PROTOCOL:
        raise AssertionError(initialized)
    if server.handle({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) != []:
        raise AssertionError('initialized notification produced a reply')
    return server


def tool(server, request_id, name, **arguments):
    replies = server.handle({
        'jsonrpc': '2.0',
        'id': request_id,
        'method': 'tools/call',
        'params': {'name': name, 'arguments': arguments},
    })
    response = next(reply for reply in replies if reply.get('id') == request_id)
    if 'error' in response:
        raise AssertionError(response)
    return response['result']


@unittest.skipUnless(sys.platform == 'win32', 'Windows discovery adapter')
class WindowsDiscoveryAdapterTests(unittest.TestCase):
    def command(self, *arguments):
        return [sys.executable, str(Path(__file__).resolve()), '--discovery-fixture', *arguments]

    def test_registration_validation_matches_host_workspace_identity_width(self):
        from runyte_context import windows as windows_native

        incarnation = 'a' * 64
        record = {
            'workspace_id': 'b' * 32,
            'root': r'C:\workspace',
            'mode': 'standalone',
            'environment': 'c' * 64,
            'endpoint': rf'\\.\pipe\runyte-context-v1-{incarnation}',
            'host_incarnation': incarnation,
            'pid': os.getpid(),
            'creation_time': 1,
        }
        windows_native.validate_record(record)
        for width in (31, 33, 64):
            with self.subTest(width=width):
                record['workspace_id'] = 'b' * width
                with self.assertRaises(OSError):
                    windows_native.validate_record(record)

    def test_bounded_success_returns_exact_bytes_and_status(self):
        from runyte_context import windows as windows_native

        data, status = windows_native.run_discovery(self.command('success'), .5, 64)
        self.assertEqual(data, b'{"bounded":true}')
        self.assertEqual(status, 0)

    def test_oversized_output_stops_at_one_byte_past_the_bound(self):
        from runyte_context import windows as windows_native

        data, status = windows_native.run_discovery(self.command('oversized', '65'), .5, 64)
        self.assertEqual(len(data), 65)
        self.assertEqual(status, 0)

    def test_timeout_terminates_the_owned_child_within_the_deadline(self):
        from runyte_context import windows as windows_native

        started_at = time.monotonic()
        with self.assertRaises(TimeoutError):
            windows_native.run_discovery(self.command('timeout', '5'), .1, 64)
        self.assertLess(time.monotonic() - started_at, 1)

    def test_descendant_held_stdout_is_cancelled_at_the_same_deadline(self):
        from runyte_context import windows as windows_native

        started_at = time.monotonic()
        with self.assertRaises(TimeoutError):
            windows_native.run_discovery(self.command('hold-stdout', '2'), .1, 64)
        self.assertLess(time.monotonic() - started_at, 1)
        time.sleep(2.1)  # Leave no checked-in fixture descendant running after this test.


@unittest.skipUnless(NATIVE_FIXTURE, 'launched by the private Windows Rust context host fixture')
class NativeWindowsBridgeTests(unittest.TestCase):
    def test_private_windows_mcp_round_trip_security_unicode_reconnect_and_revoke(self):
        record = json.loads(os.environ['RUNYTE_CONTEXT_NATIVE_RECORD'])
        root = Path(os.environ['RUNYTE_CONTEXT_NATIVE_ROOT'])
        preauth_ready = Path(os.environ['RUNYTE_CONTEXT_NATIVE_PREAUTH_READY'])
        preauth_proceed = Path(os.environ['RUNYTE_CONTEXT_NATIVE_PREAUTH_CONTINUE'])
        ready = Path(os.environ['RUNYTE_CONTEXT_NATIVE_READY'])
        proceed = Path(os.environ['RUNYTE_CONTEXT_NATIVE_CONTINUE'])
        inventory = {
            'schema': 'runyte.context.discovery.v1',
            'truncated': False,
            'workspaces': [record],
        }
        discovery = lambda hidden: inventory

        from runyte_context import windows as windows_native

        windows_native.validate_record(record)

        malformed = []
        for field, value in (
                ('pid', True), ('pid', 0), ('pid', 1.5),
                ('creation_time', False), ('creation_time', 0), ('creation_time', 1.5)):
            candidate = dict(record)
            candidate[field] = value
            malformed.append(candidate)
        for endpoint in (
                rf'\\.\pipe\other-context-v1-{record["host_incarnation"]}',
                rf'\\remote-host\pipe\runyte-context-v1-{record["host_incarnation"]}'):
            candidate = dict(record)
            candidate['endpoint'] = endpoint
            malformed.append(candidate)
        for candidate in malformed:
            with self.assertRaises(OSError):
                windows_native.validate_record(candidate)

        mismatched = dict(record)
        creation_time = record['creation_time']
        mismatched['creation_time'] = creation_time - 1 if creation_time > 1 else 2
        windows_native.validate_record(mismatched)
        mismatched_inventory = {**inventory, 'workspaces': [mismatched]}
        process_bridge = Bridge(name='agent', root=root, timeout=2,
                                discovery=lambda hidden: mismatched_inventory)
        self.addCleanup(process_bridge.close)
        process_denied = tool(started(process_bridge), 2, 'list_workspaces')['structuredContent']
        self.assertEqual(len(process_denied['workspaces']), 1, mismatched)
        self.assertFalse(process_denied['workspaces'][0]['readable'])
        self.assertEqual(process_denied['workspaces'][0]['unavailable_reason'], 'unavailable')

        linked_bridge = Bridge(name='linked', root=root, timeout=2, discovery=discovery)
        self.addCleanup(linked_bridge.close)
        linked = tool(started(linked_bridge), 3, 'list_workspaces')['structuredContent']
        self.assertFalse(linked['workspaces'][0]['readable'])
        self.assertEqual(linked['workspaces'][0]['unavailable_reason'], 'unavailable')

        preauth_ready.write_text('ready', encoding='ascii')
        deadline = time.monotonic() + 15
        while not preauth_proceed.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertTrue(preauth_proceed.exists(), 'Rust fixture did not verify pre-auth rejection')

        denied_bridge = Bridge(name='attacker', root=root, timeout=2, discovery=discovery)
        self.addCleanup(denied_bridge.close)
        denied = tool(started(denied_bridge), 4, 'list_workspaces')['structuredContent']
        self.assertEqual(len(denied['workspaces']), 1)
        self.assertFalse(denied['workspaces'][0]['readable'])
        self.assertEqual(denied['workspaces'][0]['unavailable_reason'], 'capability_denied')

        first_bridge = Bridge(name='agent', root=root, timeout=2, discovery=discovery)
        self.addCleanup(first_bridge.close)
        first = started(first_bridge)
        tools = first.handle({'jsonrpc': '2.0', 'id': 5, 'method': 'tools/list', 'params': {}})
        names = {entry['name'] for entry in tools[0]['result']['tools']}
        self.assertIn('edit_buffer', names)
        listed = tool(first, 6, 'list_workspaces')['structuredContent']
        workspace = listed['workspaces'][0]['workspace']
        self.assertTrue(listed['workspaces'][0]['readable'])
        buffers = tool(first, 7, 'list_buffers', workspace=workspace)['structuredContent']['data']['buffers']
        buffer = buffers[0]
        read = tool(first, 8, 'read_buffer', workspace=workspace, buffer=buffer['buffer'],
                    expected_revision=buffer['revision'], **{'from': 0, 'to': buffer['chars']})
        self.assertEqual(read['structuredContent']['data']['text'], 'aç界🙂z')
        edited = tool(first, 9, 'edit_buffer', workspace=workspace, buffer=buffer['buffer'],
                      expected_revision=buffer['revision'],
                      changes=[{'from': 1, 'to': 4, 'text': 'Żółć🙂'}])
        self.assertFalse(edited['isError'])
        stale = tool(first, 10, 'read_buffer', workspace=workspace, buffer=buffer['buffer'],
                     expected_revision=buffer['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(stale['isError'])
        self.assertEqual(stale['structuredContent']['error']['code'], 'stale')
        old_handle = buffer['buffer']
        first_bridge.close()

        second_bridge = Bridge(name='agent', root=root, timeout=2, discovery=discovery)
        self.addCleanup(second_bridge.close)
        second = started(second_bridge)
        relisted = tool(second, 11, 'list_workspaces')['structuredContent']
        workspace = relisted['workspaces'][0]['workspace']
        crossed = tool(second, 12, 'read_buffer', workspace=workspace, buffer=old_handle,
                       expected_revision=buffer['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(crossed['isError'])
        self.assertEqual(crossed['structuredContent']['error']['code'], 'stale')
        current = tool(second, 13, 'list_buffers', workspace=workspace)['structuredContent']['data']['buffers'][0]
        observed = tool(second, 14, 'read_buffer', workspace=workspace, buffer=current['buffer'],
                        expected_revision=current['revision'], **{'from': 0, 'to': current['chars']})
        self.assertEqual(observed['structuredContent']['data']['text'], 'aŻółć🙂z')

        ready.write_text('ready', encoding='ascii')
        deadline = time.monotonic() + 15
        while not proceed.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertTrue(proceed.exists(), 'Rust fixture did not complete grant retirement')
        revoked = tool(second, 15, 'read_buffer', workspace=workspace, buffer=current['buffer'],
                       expected_revision=current['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(revoked['isError'])
        self.assertIn(revoked['structuredContent']['error']['code'],
                      {'cancelled', 'capability_denied', 'unavailable'})
        final = tool(second, 16, 'list_workspaces')['structuredContent']
        self.assertFalse(final['workspaces'][0]['readable'])


@unittest.skipUnless(PUBLIC_FIXTURE, 'launched by the public Windows Runyte acceptance')
class PublicWindowsBridgeTests(unittest.TestCase):
    def wait_file(self, path, message):
        deadline = time.monotonic() + 30
        while not path.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertTrue(path.exists(), message)

    def readable(self, bridge, previous_incarnation=None):
        deadline = time.monotonic() + 30
        last = None
        while time.monotonic() < deadline:
            listed = tool(started(bridge), 1, 'list_workspaces')['structuredContent']
            last = listed['workspaces']
            if len(last) == 1 and last[0]['readable']:
                if previous_incarnation is None or last[0]['host_incarnation'] != previous_incarnation:
                    return started(bridge), last[0]
            bridge.close()
            time.sleep(.05)
        self.fail(f'public Runyte workspace did not become readable: {last!r}')

    def test_public_executable_discovery_edit_reconnect_remember_restart_and_revoke(self):
        binary = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_BINARY'])
        root = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_ROOT'])
        phase_ready = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_PHASE_READY'])
        phase_continue = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_PHASE_CONTINUE'])
        restart_ready = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_RESTART_READY'])
        revoke_continue = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_REVOKE_CONTINUE'])

        first_bridge = Bridge(executable=str(binary), name='agent', root=root, timeout=2)
        self.addCleanup(first_bridge.close)
        first, listed = self.readable(first_bridge)
        first_incarnation = listed['host_incarnation']
        workspace = listed['workspace']
        buffers = tool(first, 2, 'list_buffers', workspace=workspace)['structuredContent']['data']['buffers']
        buffer = next(row for row in buffers if row.get('name', '').endswith('unicode note.txt'))
        read = tool(first, 3, 'read_buffer', workspace=workspace, buffer=buffer['buffer'],
                    expected_revision=buffer['revision'], **{'from': 0, 'to': buffer['chars']})
        self.assertEqual(read['structuredContent']['data']['text'], 'a\u00e7\u754c\U0001f642z')
        edited = tool(first, 4, 'edit_buffer', workspace=workspace, buffer=buffer['buffer'],
                      expected_revision=buffer['revision'],
                      changes=[{'from': 1, 'to': 4, 'text': '\u017b\u00f3\u0142\u0107\U0001f642'}])
        self.assertFalse(edited['isError'])
        stale = tool(first, 5, 'read_buffer', workspace=workspace, buffer=buffer['buffer'],
                     expected_revision=buffer['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(stale['isError'])
        self.assertEqual(stale['structuredContent']['error']['code'], 'stale')
        old_handle = buffer['buffer']
        first_bridge.close()

        second_bridge = Bridge(executable=str(binary), name='agent', root=root, timeout=2)
        self.addCleanup(second_bridge.close)
        second, listed = self.readable(second_bridge)
        workspace = listed['workspace']
        crossed = tool(second, 6, 'read_buffer', workspace=workspace, buffer=old_handle,
                       expected_revision=buffer['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(crossed['isError'])
        self.assertEqual(crossed['structuredContent']['error']['code'], 'stale')
        current = next(row for row in
                       tool(second, 7, 'list_buffers', workspace=workspace)['structuredContent']['data']['buffers']
                       if row.get('name', '').endswith('unicode note.txt'))
        observed = tool(second, 8, 'read_buffer', workspace=workspace, buffer=current['buffer'],
                        expected_revision=current['revision'], **{'from': 0, 'to': current['chars']})
        self.assertEqual(observed['structuredContent']['data']['text'], 'a\u017b\u00f3\u0142\u0107\U0001f642z')
        second_bridge.close()

        phase_ready.write_text('ready', encoding='ascii')
        self.wait_file(phase_continue, 'Rust fixture did not restart the remembered grant')

        restarted_bridge = Bridge(executable=str(binary), name='agent', root=root, timeout=2)
        self.addCleanup(restarted_bridge.close)
        restarted, listed = self.readable(restarted_bridge, first_incarnation)
        workspace = listed['workspace']
        current = next(row for row in
                       tool(restarted, 9, 'list_buffers', workspace=workspace)['structuredContent']['data']['buffers']
                       if row.get('name', '').endswith('unicode note.txt'))
        observed = tool(restarted, 10, 'read_buffer', workspace=workspace, buffer=current['buffer'],
                        expected_revision=current['revision'], **{'from': 0, 'to': current['chars']})
        self.assertEqual(observed['structuredContent']['data']['text'], 'a\u00e7\u754c\U0001f642z')

        restart_ready.write_text('ready', encoding='ascii')
        self.wait_file(revoke_continue, 'Rust fixture did not revoke the remembered grant')
        revoked = tool(restarted, 11, 'read_buffer', workspace=workspace, buffer=current['buffer'],
                       expected_revision=current['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(revoked['isError'])
        self.assertIn(revoked['structuredContent']['error']['code'],
                      {'cancelled', 'capability_denied', 'unavailable'})
        final = tool(restarted, 12, 'list_workspaces')['structuredContent']
        self.assertEqual(final['workspaces'], [])


if __name__ == '__main__':
    unittest.main()
