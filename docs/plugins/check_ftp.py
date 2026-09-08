# SPDX-License-Identifier: MPL-2.0
"""Real local FTP/FTPS behavior checks; no live accounts or stored credentials."""
import concurrent.futures
import ftplib
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import threading
import unittest
from unittest.mock import patch

from application import PluginError
from download_transport_checks import DownloadTransportChecks
from download_wire_checks import check_download_wire
from operation_transport_checks import OperationTransportChecks
from ftp_fixture import FtpFixture
from ftp_transport import FtpTransport

DIRECTORY = Path(__file__).resolve().parent


def version(data):
    return hashlib.sha256(data).hexdigest()


class FtpTransportTests(OperationTransportChecks, DownloadTransportChecks, unittest.TestCase):
    def fixture(self, **options):
        fixture = FtpFixture(**options)
        self.addCleanup(fixture.close)
        return fixture

    def transport(self, fixture, **options):
        return FtpTransport({**fixture.config, **options})

    def assert_no_destination_delete(self, fixture, destination):
        self.assertFalse(any(op[0] == 'DELE' and op[1] == str(destination)
                             for op in fixture.operations), fixture.operations)

    def test_verified_tls_reads_unicode_empty_and_raw_binary(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        for name, data in [('猫.txt', 'é猫 🦀\r\n'.encode()), ('empty', b''), ('binary', b'\x00\xff')]:
            (fixture.root / name).write_bytes(data)
            self.assertEqual(transport.read(name), data)
        self.assertEqual([op[1] for op in fixture.operations if op[0] == 'PROT'], ['P'] * 3)

    def test_explicit_plain_ftp_namespace_operations_preserve_the_selected_protocol(self):
        fixture = self.fixture(tls=False)
        transport = self.transport(fixture)
        transport.apply_operation(transport.prepare_operation('mkdir', 'new'))
        self.assertTrue((fixture.root / 'new').is_dir())
        transport.apply_operation(transport.prepare_operation('delete', 'new'))
        self.assertFalse((fixture.root / 'new').exists())
        self.assertFalse(any(op[0] == 'PROT' for op in fixture.operations))

    def test_document_limit_and_regular_file_requirement(self):
        fixture = self.fixture()
        oversized = fixture.root / 'oversized'
        with oversized.open('wb') as stream:
            stream.truncate(8 * 1024 * 1024 + 1)
        transport = self.transport(fixture)
        with self.assertRaises(PluginError) as raised:
            transport.read('oversized')
        self.assertEqual(raised.exception.code, 'limit_exceeded')
        (fixture.root / 'directory').mkdir()
        with self.assertRaises(PluginError):
            transport.read('directory')

    def test_root_containment_rejects_traversal_and_absolute_escape(self):
        fixture = self.fixture()
        (fixture.root / 'project').mkdir()
        (fixture.root / 'project' / 'notes').write_text('inside')
        (fixture.root / 'outside').write_text('outside')
        transport = self.transport(fixture, root='/project')
        self.assertEqual(transport.read('notes'), b'inside')
        for path in ['../outside', '/outside', '//outside', 'notes\nRETR /outside']:
            with self.subTest(path=path), self.assertRaises(PluginError):
                transport.read(path)

    def test_mlsd_returns_unicode_entries_and_enforces_entry_bound(self):
        fixture = self.fixture()
        (fixture.root / '猫.txt').write_bytes('é'.encode())
        (fixture.root / 'directory').mkdir()
        transport = self.transport(fixture)
        with patch.object(ftplib.FTP, 'mlsd', side_effect=AssertionError('Unbounded MLSD collection')):
            canonical, entries = transport.browse('.')
        self.assertEqual(canonical, '/')
        self.assertIn({'name': '猫.txt', 'kind': 'file', 'size': 2}, entries)
        self.assertTrue(any(row['name'] == 'directory' and row['kind'] == 'directory' for row in entries))
        for number in range(1023):
            (fixture.root / f'entry{number:04}').touch()
        with self.assertRaises(PluginError) as raised:
            transport.browse('.')
        self.assertEqual(raised.exception.code, 'limit_exceeded')

    def test_untrusted_certificate_and_wrong_hostname_never_transfer_data(self):
        for mode in ['untrusted', 'hostname']:
            with self.subTest(mode=mode):
                fixture = self.fixture(valid_hostname=mode != 'hostname')
                (fixture.root / 'notes').write_text('secret')
                config = dict(fixture.config)
                if mode == 'untrusted':
                    config.pop('ca_file')
                with self.assertRaises(PluginError):
                    FtpTransport(config).read('notes')
                self.assertFalse(any(op[0] in ('PROT', 'RETR') for op in fixture.operations))

    def test_authentication_and_private_data_protection_refusal_do_not_fallback(self):
        for mode in ['password', 'protection']:
            with self.subTest(mode=mode):
                fixture = self.fixture(reject_protection=mode == 'protection')
                (fixture.root / 'notes').write_text('secret')
                if mode == 'password':
                    fixture.password_file.write_text('incorrect\n')
                with self.assertRaises(PluginError):
                    self.transport(fixture).read('notes')
                self.assertFalse(any(op[0] == 'RETR' for op in fixture.operations))
                self.assertFalse(any(op == ('PROT', 'C') for op in fixture.operations))

    def test_explicit_plain_ftp_works_and_ftps_never_downgrades(self):
        fixture = self.fixture(tls=False)
        (fixture.root / 'notes').write_text('plain transport was selected')
        transport = self.transport(fixture)
        self.assertEqual(transport.read('notes'), b'plain transport was selected')
        transport.replace('notes', b'explicit plain save', version(b'plain transport was selected'))
        self.assertEqual((fixture.root / 'notes').read_bytes(), b'explicit plain save')
        self.assertFalse(any(op[0] == 'PROT' for op in fixture.operations))
        before = len(fixture.operations)
        with self.assertRaises(PluginError):
            self.transport(fixture, transport='ftps').read('notes')
        self.assertFalse(any(op[0] == 'RETR' for op in fixture.operations[before:]))

    def test_private_password_file_rejects_symlink_permissions_and_invalid_content(self):
        fixture = self.fixture()
        link = fixture.base / 'password-link'
        link.symlink_to(fixture.password_file)
        fixtures = [(link, None), (fixture.base, None)]
        for number, value in enumerate([b'x' * 4097, b'line\nline', b'\xff', b'']):
            path = fixture.base / f'invalid{number}'
            path.write_bytes(value)
            path.chmod(0o600)
            fixtures.append((path, None))
        public = fixture.base / 'public-password'
        public.write_text(fixture.password)
        public.chmod(0o644)
        fixtures.append((public, None))
        for path, _ in fixtures:
            with self.subTest(path=path.name), self.assertRaises(PluginError):
                self.transport(fixture, password_file=str(path)).read('notes')
        self.assertFalse(any(op[0] == 'RETR' for op in fixture.operations))

    def test_utf8_password_accepts_crlf_and_connection_identity_omits_secret_paths(self):
        fixture = self.fixture()
        (fixture.root / 'notes').write_text('text')
        fixture.password_file.write_bytes((fixture.password + '\r\n').encode())
        first = self.transport(fixture)
        self.assertEqual(first.read('notes'), b'text')
        second_file = fixture.base / 'other-password'
        second_file.write_text(fixture.password)
        second_file.chmod(0o600)
        self.assertEqual(first.connection_id, self.transport(fixture, password_file=str(second_file)).connection_id)
        self.assertNotIn(str(fixture.base), first.connection_id)

    def test_replacement_stages_then_renames_without_destination_delete(self):
        fixture = self.fixture()
        destination = fixture.root / 'notes'
        destination.write_bytes(b'base')
        data = '猫\r\né'.encode()
        self.assertEqual(self.transport(fixture).replace('/notes', data, version(b'base')), version(data))
        self.assertEqual(destination.read_bytes(), data)
        staging = [Path(op[1]) for op in fixture.operations if op[0] == 'STOR']
        self.assertEqual(len(staging), 1)
        self.assertTrue(staging[0].parent.name.startswith('.runyte-upload-'))
        self.assert_no_destination_delete(fixture, destination)
        self.assertEqual(list(fixture.root.iterdir()), [destination])
        self.assertEqual(self.transport(fixture).replace('/notes', b'', version(data)), version(b''))
        self.assertEqual(destination.read_bytes(), b'')
        self.assert_no_destination_delete(fixture, destination)

    def test_hash_preflight_detects_equal_size_equal_timestamp_external_edit(self):
        fixture = self.fixture()
        destination = fixture.root / 'notes'
        destination.write_bytes(b'base')
        original = destination.stat()
        def changed(_):
            destination.write_bytes(b'edit')
            os.utime(destination, ns=(original.st_atime_ns, original.st_mtime_ns))
        fixture.on_write_close = changed
        with self.assertRaises(PluginError) as raised:
            self.transport(fixture).replace('/notes', b'ours', version(b'base'))
        self.assertEqual(raised.exception.code, 'conflict')
        self.assertEqual(destination.read_bytes(), b'edit')
        self.assertFalse(fixture.rename_entered.is_set())
        self.assert_no_destination_delete(fixture, destination)

    def test_cancellation_before_promotion_keeps_destination(self):
        fixture = self.fixture()
        destination = fixture.root / 'notes'
        destination.write_bytes(b'base')
        cancel = threading.Event()
        fixture.on_write_close = lambda _: cancel.set()
        with self.assertRaises(PluginError) as raised:
            self.transport(fixture).replace('/notes', b'ours', version(b'base'), cancel)
        self.assertEqual(raised.exception.code, 'cancelled')
        self.assertEqual(destination.read_bytes(), b'base')
        self.assertFalse(fixture.rename_entered.is_set())
        self.assert_no_destination_delete(fixture, destination)

    def test_drop_after_rnto_is_unknown_and_never_retried(self):
        fixture = self.fixture()
        destination = fixture.root / 'notes'
        destination.write_bytes(b'base')
        fixture.drop_rename_reply = True
        with self.assertRaises(PluginError) as raised:
            self.transport(fixture).replace('/notes', b'ours', version(b'base'))
        self.assertEqual(raised.exception.code, 'outcome_unknown')
        self.assertFalse(raised.exception.settled)
        self.assertEqual(destination.read_bytes(), b'ours')
        self.assertEqual(sum(op[0] == 'RNTO' for op in fixture.operations), 1)
        self.assert_no_destination_delete(fixture, destination)

    def test_unsupported_rename_never_deletes_existing_destination(self):
        fixture = self.fixture(reject_rename=True)
        destination = fixture.root / 'notes'
        destination.write_bytes(b'base')
        with self.assertRaises(PluginError):
            self.transport(fixture).replace('/notes', b'ours', version(b'base'))
        self.assertEqual(destination.read_bytes(), b'base')
        self.assertEqual(sum(op[0] == 'RNTO' for op in fixture.operations), 1)
        self.assert_no_destination_delete(fixture, destination)

    def test_remote_write_between_preflight_and_rename_demonstrates_weak_guarantee(self):
        fixture = self.fixture()
        destination = fixture.root / 'notes'
        destination.write_bytes(b'base')
        fixture.before_rename = lambda source, target: target.write_bytes(b'external')
        self.transport(fixture).replace('/notes', b'ours', version(b'base'))
        self.assertEqual(destination.read_bytes(), b'ours')
        self.assert_no_destination_delete(fixture, destination)

    def test_two_blocked_reads_bound_admission_and_cancellation_releases_transport(self):
        fixture = self.fixture()
        (fixture.root / 'notes').write_text('base')
        fixture.read_release.clear()
        transport = self.transport(fixture)
        cancelled = [threading.Event(), threading.Event()]
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            pending = [executor.submit(transport.read, 'notes', cancel) for cancel in cancelled]
            try:
                self.assertTrue(fixture.two_reads_started.wait(2))
                with self.assertRaises(PluginError) as raised:
                    transport.read('notes')
                self.assertEqual(raised.exception.code, 'busy')
                for cancel in cancelled:
                    cancel.set()
                for future in pending:
                    with self.assertRaises(PluginError) as raised:
                        future.result(timeout=2)
                    self.assertEqual(raised.exception.code, 'cancelled')
                    self.assertTrue(raised.exception.settled)
            finally:
                fixture.read_release.set()
                for cancel in cancelled:
                    cancel.set()
        self.assertFalse(fixture.rename_entered.is_set())

    def test_blocked_successful_rename_reply_times_out_as_unknown(self):
        fixture = self.fixture()
        destination = fixture.root / 'notes'
        destination.write_bytes(b'base')
        fixture.rename_release.clear()
        with patch('ftp_transport.OPERATION_TIMEOUT', 0.6):
            with self.assertRaises(PluginError) as raised:
                self.transport(fixture).replace('/notes', b'ours', version(b'base'))
        self.assertTrue(fixture.rename_done.is_set())
        self.assertEqual(raised.exception.code, 'outcome_unknown')
        self.assertFalse(raised.exception.settled)
        self.assertEqual(destination.read_bytes(), b'ours')
        self.assert_no_destination_delete(fixture, destination)

    def test_checked_in_ftps_app_browses_and_reads_through_public_wire(self):
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
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'ftp.py'), '--config', str(config)],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)

        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()

        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(5), 'FTPS app response timed out')
            data = child.stdout.readline(1024 * 1024 + 1)
            self.assertTrue(data.endswith(b'\n'), 'FTPS app disconnected or returned incomplete frame')
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
            self.assertFalse(register['params']['atomic_replace'])
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
            check_download_wire(self, fixture, invocation, row, contents, send, receive, reply)
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
