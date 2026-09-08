# SPDX-License-Identifier: MPL-2.0
"""Actual local SSH/SFTP contract checks; no network service or account needed."""

import hashlib
import concurrent.futures
import json
import os
from pathlib import Path
import selectors
import stat
import subprocess
import sys
import threading
import unittest

import paramiko

from application import PluginError
from sftp_fixture import SftpFixture
from sftp_transport import SftpTransport

DIRECTORY = Path(__file__).resolve().parent


def version(data):
    return hashlib.sha256(data).hexdigest()


class SftpTransportTests(unittest.TestCase):
    def fixture(self, **options):
        fixture = SftpFixture(**options)
        self.addCleanup(fixture.close)
        return fixture

    def transport(self, fixture, **options):
        transport = SftpTransport({**fixture.config, **options})
        if hasattr(transport, 'close'):
            self.addCleanup(transport.close)
        return transport

    def assert_no_destination_delete(self, fixture, path):
        self.assertFalse(any(operation[0] == 'remove' and operation[1] == str(path)
                             for operation in fixture.operations), fixture.operations)
        self.assertFalse(any(operation[0] == 'rename' for operation in fixture.operations),
                         'A basic rename fallback was attempted')

    def test_read_unicode_empty_and_binary_bytes(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        for name, contents in [('unicode.txt', 'é猫 🦀\r\n'.encode()),
                               ('empty.txt', b''), ('binary.bin', b'\x00\xff\xfe')]:
            (fixture.root / name).write_bytes(contents)
            with self.subTest(name=name):
                self.assertEqual(transport.read(name), contents)

    def test_read_document_limit_and_regular_file_requirement(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        path = fixture.root / 'oversized.txt'
        with path.open('wb') as stream:
            stream.truncate(8 * 1024 * 1024 + 1)
        with self.assertRaises(PluginError) as raised:
            transport.read(path.name)
        self.assertEqual(raised.exception.code, 'limit_exceeded')
        directory = fixture.root / 'directory'
        directory.mkdir()
        with self.assertRaises(PluginError):
            transport.read(directory.name)

    def test_canonical_paths_reject_workspace_escape_and_outside_symlink(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        outside = fixture.base / 'outside.txt'
        outside.write_text('outside')
        (fixture.root / 'outside-link').symlink_to(outside)
        for path in ['../outside.txt', str(outside), 'outside-link']:
            with self.subTest(path=path), self.assertRaises(PluginError):
                transport.read(path)
        child = fixture.root / 'child'
        child.mkdir()
        (child / 'note').write_text('inside')
        (fixture.root / 'inside-link').symlink_to(child)
        self.assertEqual(transport.read('inside-link/note'), b'inside')

    def test_list_is_bounded_and_retains_unicode_kind_and_size(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        (fixture.root / 'é.txt').write_bytes('猫'.encode())
        (fixture.root / 'directory').mkdir()
        (fixture.root / 'link').symlink_to(fixture.root / 'é.txt')
        canonical, entries = transport.browse('.')
        self.assertEqual(canonical, str(fixture.root))
        by_name = {entry['name']: entry for entry in entries}
        self.assertEqual(by_name['é.txt']['kind'], 'file')
        self.assertEqual(by_name['é.txt']['size'], 3)
        self.assertEqual(by_name['directory']['kind'], 'directory')
        self.assertEqual(by_name['link']['kind'], 'symlink')
        for index in range(1022):
            (fixture.root / f'entry-{index:04}').touch()
        with self.assertRaises(PluginError) as raised:
            transport.browse('.')
        self.assertEqual(raised.exception.code, 'limit_exceeded')

    def test_replace_uses_staging_and_posix_rename_without_destination_delete(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        original, replacement = b'old\n', 'é猫 🦀\n'.encode()
        destination.write_bytes(original)
        destination.chmod(0o640)
        saved = transport.replace(destination.name, replacement, version(original), threading.Event())
        self.assertEqual(saved, version(replacement))
        self.assertEqual(destination.read_bytes(), replacement)
        self.assertEqual(stat.S_IMODE(destination.stat().st_mode), 0o640)
        promotions = [operation for operation in fixture.operations
                      if operation[0] == 'posix_rename'
                      and Path(operation[1]).name.startswith('.runyte-upload-')]
        self.assertEqual(len(promotions), 1)
        self.assertNotEqual(promotions[0][1], str(destination))
        self.assertEqual(Path(promotions[0][1]).parent, destination.parent)
        self.assertEqual(promotions[0][2], str(destination))
        self.assert_no_destination_delete(fixture, destination)
        self.assertEqual(list(fixture.root.iterdir()), [destination])

    def test_missing_posix_extension_never_falls_back_to_delete_or_basic_rename(self):
        fixture = self.fixture(posix_rename=False)
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        destination.write_bytes(b'old')
        with self.assertRaises(PluginError) as raised:
            transport.replace(destination.name, b'new', version(b'old'), threading.Event())
        self.assertEqual(raised.exception.code, 'unsupported')
        self.assertTrue(getattr(raised.exception, 'settled', False))
        self.assertFalse(fixture.rename_entered.is_set())
        self.assertFalse(any(operation[0] == 'open'
                             and Path(operation[1]).name.startswith('.runyte-upload-')
                             for operation in fixture.operations))
        self.assertEqual(destination.read_bytes(), b'old')
        self.assert_no_destination_delete(fixture, destination)

    def test_hash_preflight_rejects_same_size_same_timestamp_external_edit(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        original, external = b'original', b'external'
        destination.write_bytes(original)
        timestamp = destination.stat()

        def edit_after_staging(_path):
            destination.write_bytes(external)
            os.utime(destination, ns=(timestamp.st_atime_ns, timestamp.st_mtime_ns))

        fixture.on_write_close = edit_after_staging
        with self.assertRaises(PluginError) as raised:
            transport.replace(destination.name, b'new text', version(original), threading.Event())
        self.assertEqual(raised.exception.code, 'conflict')
        self.assertFalse(fixture.rename_entered.is_set())
        self.assertEqual(destination.read_bytes(), external)
        self.assert_no_destination_delete(fixture, destination)

    def test_stale_initial_hash_refuses_promotion(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        destination.write_bytes(b'external')
        with self.assertRaises(PluginError) as raised:
            transport.replace(destination.name, b'new', version(b'previous'), threading.Event())
        self.assertEqual(raised.exception.code, 'conflict')
        self.assertFalse(fixture.rename_entered.is_set())
        self.assertEqual(destination.read_bytes(), b'external')

    def test_cancel_before_promotion_never_mutates_destination(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        destination.write_bytes(b'old')
        cancelled = threading.Event()
        fixture.on_write_close = lambda _path: cancelled.set()
        with self.assertRaises(PluginError) as raised:
            transport.replace(destination.name, b'new', version(b'old'), cancelled)
        self.assertEqual(raised.exception.code, 'cancelled')
        self.assertFalse(fixture.rename_entered.is_set())
        self.assertEqual(destination.read_bytes(), b'old')
        self.assert_no_destination_delete(fixture, destination)
        for operation in fixture.operations:
            if operation[0] == 'remove':
                self.assertEqual(Path(operation[1]).parent, destination.parent)
                self.assertTrue(Path(operation[1]).name.startswith(('.runyte-upload-', '.runyte-probe-')))

    def test_dropped_successful_rename_reply_is_unknown(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        destination.write_bytes(b'old')
        fixture.drop_rename_reply = True
        with self.assertRaises(PluginError) as raised:
            transport.replace(destination.name, b'new', version(b'old'), threading.Event())
        self.assertEqual(raised.exception.code, 'outcome_unknown')
        self.assertTrue(fixture.rename_done.is_set())
        self.assertEqual(destination.read_bytes(), b'new')
        self.assert_no_destination_delete(fixture, destination)

    def test_blocked_rename_reply_times_out_without_claiming_rollback(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        destination.write_bytes(b'old')
        fixture.rename_release.clear()
        try:
            with self.assertRaises(PluginError) as raised:
                transport.replace(destination.name, b'new', version(b'old'), threading.Event())
            self.assertEqual(raised.exception.code, 'outcome_unknown')
            self.assertTrue(fixture.rename_done.is_set())
            self.assertEqual(destination.read_bytes(), b'new')
            self.assert_no_destination_delete(fixture, destination)
        finally:
            fixture.rename_release.set()

    def test_best_effort_promotion_does_not_claim_compare_and_swap(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        destination = fixture.root / 'note.txt'
        destination.write_bytes(b'old')
        fixture.before_rename = lambda _source, target: target.write_bytes(b'external race')
        self.assertEqual(transport.replace(destination.name, b'new', version(b'old'), threading.Event()),
                         version(b'new'))
        # This intentional race explains conditional_write=False in the adapter.
        self.assertEqual(destination.read_bytes(), b'new')

    def test_host_key_mismatch_does_not_reach_sftp(self):
        fixture = self.fixture()
        wrong_key = paramiko.RSAKey.generate(2048)
        fixture.known_hosts.write_text(
            f'[127.0.0.1]:{fixture.port} {wrong_key.get_name()} {wrong_key.get_base64()}\n')
        with self.assertRaises(PluginError):
            self.transport(fixture).read('note.txt')
        self.assertEqual(fixture.operations, [])

    def test_unknown_host_key_and_rejected_identity_do_not_reach_sftp(self):
        for unknown in [False, True]:
            fixture = self.fixture(accept_client=unknown)
            if unknown:
                fixture.known_hosts.write_text('')
            with self.subTest(unknown_host=unknown), self.assertRaises(PluginError):
                self.transport(fixture).read('note.txt')
            self.assertEqual(fixture.operations, [])

    def test_unicode_controls_are_rejected_in_configuration_and_paths(self):
        fixture = self.fixture()
        with self.assertRaises(PluginError) as raised:
            self.transport(fixture, alias='Hidden\u0085control')
        self.assertEqual(raised.exception.code, 'invalid_argument')
        with self.assertRaises(PluginError) as raised:
            self.transport(fixture).read('note\u0085.txt')
        self.assertEqual(raised.exception.code, 'invalid_argument')

    def test_two_blocked_reads_bound_admission_without_a_waiting_queue(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        (fixture.root / 'note').write_bytes(b'bounded')
        fixture.read_release.clear()
        executor = concurrent.futures.ThreadPoolExecutor(max_workers=2)
        pending = [executor.submit(transport.read, 'note') for _ in range(2)]
        try:
            self.assertTrue(fixture.two_reads_started.wait(3), 'Both reads did not reach the server')
            with self.assertRaises(PluginError) as raised:
                transport.read('note')
            self.assertEqual(raised.exception.code, 'busy')
            fixture.read_release.set()
            for future in pending:
                self.assertEqual(future.result(timeout=3), b'bounded')
        finally:
            fixture.read_release.set()
            executor.shutdown(wait=True, cancel_futures=True)

    def test_cancel_interrupts_blocked_read_before_any_promotion(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        (fixture.root / 'note').write_bytes(b'unchanged')
        fixture.read_release.clear()
        cancelled = threading.Event()
        executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)
        pending = executor.submit(transport.read, 'note', cancelled)
        try:
            self.assertTrue(fixture.read_started.wait(3), 'Read did not reach the server')
            cancelled.set()
            with self.assertRaises(PluginError) as raised:
                pending.result(timeout=2)
            self.assertEqual(raised.exception.code, 'cancelled')
            self.assertFalse(getattr(raised.exception, 'outcome_unknown', False))
            self.assertFalse(fixture.rename_entered.is_set())
        finally:
            fixture.read_release.set()
            executor.shutdown(wait=True, cancel_futures=True)

    def test_checked_in_sftp_app_browses_and_reads_through_public_wire(self):
        from jsonschema import Draft202012Validator
        fixture = self.fixture()
        contents = 'é猫 🦀\r\n'.encode()
        (fixture.root / 'é.txt').write_bytes(contents)
        config = fixture.base / 'profile.json'
        config.write_text(json.dumps(fixture.config))
        schema = json.loads((DIRECTORY / 'runyte-experimental-2.schema.json').read_text())
        validator = Draft202012Validator({**schema, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]})
        host = [entry['message'] for entry in
                json.loads((DIRECTORY / 'epoch2-fixtures.json').read_text())
                if entry['direction'] == 'host']
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'sftp.py'), '--config', str(config)],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)

        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()

        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(5), 'SFTP app response timed out')
            data = child.stdout.readline(1024 * 1024 + 1)
            self.assertTrue(data.endswith(b'\n'), 'SFTP app disconnected or returned incomplete frame')
            message = json.loads(data)
            validator.validate(message)
            return message

        def reply(request, result):
            send({'type': 'response', 'id': request['id'], 'result': result})

        try:
            send(host[0])
            registration = receive()
            self.assertIn('providers', registration['required_capabilities'])
            send({**host[1], 'capabilities': ['views', 'providers', 'documents', 'jobs']})
            invocation = {**host[2], 'params': {**host[2]['params'], 'command': 'browse',
                                                'arguments': {'path': '.'}}}
            send(invocation)
            register = receive()
            self.assertEqual(register['method'], 'provider.register')
            self.assertFalse(register['params']['conditional_write'])
            self.assertTrue(register['params']['atomic_replace'])
            reply(register, {})
            create = receive()
            self.assertEqual(create['method'], 'view.create')
            model = create['params']['model']
            row = next(row for row in model['rows'] if row['text'] == 'é.txt')
            reply(create, {'view': 'v:g:1', 'revision': 'm:1', 'model': model})
            show = receive()
            self.assertEqual(show['method'], 'pane.show')
            reply(show, {})
            self.assertEqual(receive(), {'type': 'response', 'id': invocation['id'],
                                         'result': {'job': None}})
            send({'type': 'request', 'id': 'h:2', 'method': 'command.invoke', 'params': {
                **invocation['params'], 'command': 'enter', 'context': 'view', 'arguments': {},
                'view': 'v:g:1', 'model_revision': 'm:1', 'rows': [row['id']]}})
            opening = receive()
            self.assertEqual(opening['method'], 'resource.open')
            self.assertEqual(opening['params']['provider'], 'remote')
            reply(opening, {'job': 'j:g:open', 'title': 'Open remote', 'state': 'running', 'progress': 0})
            self.assertEqual(receive(), {'type': 'response', 'id': 'h:2', 'result': {'job': 'j:g:open'}})
            send({'type': 'request', 'id': 'h:3', 'method': 'resource.stat', 'params': {
                'job': 'j:g:open', 'provider': 'remote', 'key': opening['params']['key']}})
            metadata = receive()['result']['value']
            self.assertEqual(metadata['bytes'], len(contents))
            send({'type': 'request', 'id': 'h:4', 'method': 'resource.read', 'params': {
                'job': 'j:g:open', 'provider': 'remote', 'key': metadata['key'],
                'version': metadata['version'], 'offset': 0, 'limit': 131072}})
            chunk = receive()['result']['value']
            self.assertEqual(chunk['text'].encode(), contents)
            self.assertTrue(chunk['eof'])
            send({'type': 'event', 'sequence': 'e:1', 'event': 'resource.released',
                  'data': {'job': 'j:g:open'}})
            child.stdin.close()
            self.assertEqual(child.wait(timeout=3), 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)
            child.stdout.close()
            child.stderr.close()
            if not child.stdin.closed:
                child.stdin.close()


if __name__ == '__main__':
    unittest.main(verbosity=2)
