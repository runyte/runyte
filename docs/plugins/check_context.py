# SPDX-License-Identifier: MPL-2.0
"""Network-free context SDK/schema conformance using local fake Unix hosts."""
import copy
import json
import os
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest

from jsonschema import Draft202012Validator
from context_client import ContextClient, ContextError, FEATURE, FRAME_BYTES, SCOPES, validate_request

DIRECTORY = Path(__file__).resolve().parent
SCHEMA = json.loads((DIRECTORY / 'runyte-context-1.schema.json').read_text())


class Host:
    def __init__(self, callback=None, *, scopes=('terminal_read', 'editor_context_read'), host_version='0.3.0', auth_error=None):
        self.directory = tempfile.TemporaryDirectory(prefix='ryctx-', dir='/tmp')
        self.path = str(Path(self.directory.name) / 'host.sock')
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(self.path)
        os.chmod(self.path, 0o600)
        self.listener.listen(1)
        self.listener.settimeout(3)
        self.frames = []
        self.callback = callback
        self.scopes = list(scopes)
        self.host_version = host_version
        self.auth_error = auth_error
        self.connection = None
        self.errors = []
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    @staticmethod
    def send(connection, value):
        connection.sendall(json.dumps(value).encode() + b'\n')

    def run(self):
        try:
            connection, _ = self.listener.accept()
            self.connection = connection
            connection.settimeout(3)
            reader = connection.makefile('rb')
            with connection, reader:
                auth = json.loads(reader.readline())
                self.frames.append(auth)
                if self.auth_error is not None:
                    self.send(connection, self.auth_error)
                    return
                self.send(connection, {'type': 'hello', 'version': 'runyte-1', 'host_version': self.host_version,
                                       'features': [FEATURE], 'capabilities': list(SCOPES), 'limits': {'line_bytes': FRAME_BYTES},
                                       'workspace': {'host_incarnation': 'a' * 64}})
                raw = reader.readline()
                if not raw:
                    return
                self.frames.append(json.loads(raw))
                self.send(connection, {'type': 'registered', 'runyte': '>=0.3.0, <0.4.0', 'features': [FEATURE],
                                       'commands': [], 'capabilities': self.scopes, 'limits': {'line_bytes': FRAME_BYTES}})
                while raw := reader.readline():
                    request = json.loads(raw)
                    self.frames.append(request)
                    if self.callback is not None:
                        if not self.callback(self, connection, request):
                            break
                    else:
                        self.send(connection, {'type': 'response', 'id': request['id'], 'result': {'terminals': [], 'next': None}})
        except (OSError, ValueError):
            # Fake malformed/closed-host tests intentionally terminate the peer.
            pass
        except Exception as error:
            self.errors.append(error)

    def close(self):
        self.listener.close()
        if self.connection is not None:
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(3)
        self.directory.cleanup()
        if self.errors:
            raise self.errors[0]

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


class ContextClientTests(unittest.TestCase):
    def client(self, host, **kwargs):
        return ContextClient(host.path, '1' * 64, **kwargs)

    def test_native_grant_handshake_explicit_read_and_close(self):
        with Host() as host, self.client(host, expected_incarnation='a' * 64) as client:
            self.assertEqual(client.capabilities, {'terminal_read', 'editor_context_read'})
            self.assertEqual(client.request('terminal.list', {'offset': 0, 'limit': 10}), {'terminals': [], 'next': None})
            self.assertEqual(host.frames[0], {'type': 'authenticate', 'credential': '1' * 64})
            self.assertEqual(host.frames[1]['commands'], [])
            self.assertEqual(host.frames[1]['required_features'], [FEATURE])
            self.assertNotIn('credential', repr(client))
        self.assertIsNone(client._socket)

    def test_ungranted_edits_and_terminal_controls_never_reach_socket(self):
        with Host() as host, self.client(host) as client:
            for method, params in [('buffer.edit', {'buffer': 'b:1', 'expected_revision': 'r:1', 'changes': [{'from': 0, 'to': 0, 'text': 'line\n'}]}),
                                   ('terminal.input', {}), ('terminal.input.approve', {}),
                                   ('terminal.list', {'offset': 0, 'limit': 10, 'submit': True})]:
                with self.subTest(method=method), self.assertRaises(ContextError):
                    client.request(method, params)
            self.assertEqual(len(host.frames), 2)

    def test_explicit_error_preserves_connection_and_error_code(self):
        def reply(host, connection, request):
            if request['id'] == 'c:1':
                host.send(connection, {'type': 'response', 'id': request['id'], 'error': {'code': 'stale', 'message': 'Buffer changed'}})
            else:
                host.send(connection, {'type': 'response', 'id': request['id'], 'result': {}})
            return True
        scopes = ('terminal_read', 'editor_context_read', 'buffer_edit')
        with Host(reply, scopes=scopes) as host, self.client(host, required_scopes=scopes, optional_scopes=()) as client:
            with self.assertRaises(ContextError) as error:
                client.request('buffer.edit', {'buffer': 'b:1', 'expected_revision': 'r:1', 'changes': [{'from': 0, 'to': 0, 'text': 'multiline\n'}]})
            self.assertEqual(error.exception.code, 'stale')
            self.assertEqual(client.request('terminal.list', {'offset': 0, 'limit': 1}), {})

    def test_uncertain_write_is_never_retried(self):
        scopes = ('terminal_read', 'terminal_propose')
        with Host(lambda *_: False, scopes=scopes) as host, self.client(host, required_scopes=scopes, optional_scopes=()) as client:
            with self.assertRaises(ContextError) as error:
                client.request('terminal.input.propose', {'terminal': 't:1', 'text': 'cargo test', 'reason': 'Run tests'})
            self.assertEqual(error.exception.code, 'outcome_unknown')
            self.assertEqual(len(host.frames), 3)
            self.assertIsNone(client._socket)

    def test_malformed_duplicate_or_mismatched_reply_closes_connection(self):
        for raw in [b'{"type":"response","id":"c:1","id":"c:1","result":{}}\n',
                    b'{"type":"response","id":"c:99","result":{}}\n',
                    b'{"type":"response","id":"c:1","result":{},"error":{}}\n',
                    b'x' * (FRAME_BYTES + 1)]:
            def reply(_, connection, request):
                connection.sendall(raw)
                return False
            with self.subTest(raw=raw[:64]), Host(reply) as host, self.client(host) as client:
                with self.assertRaises(ContextError) as error:
                    client.request('terminal.list', {'offset': 0, 'limit': 1})
                self.assertEqual(error.exception.code, 'unavailable')
                self.assertIsNone(client._socket)

    def test_deadline_covers_entire_slow_reply(self):
        def reply(_, connection, request):
            for _ in range(30):
                connection.sendall(b' ')
                time.sleep(.02)
            return False
        with Host(reply) as host, self.client(host, timeout=.1) as client:
            started = time.monotonic()
            with self.assertRaises(ContextError):
                client.request('terminal.list', {'offset': 0, 'limit': 1})
            self.assertLess(time.monotonic() - started, .5)

    def test_restart_wrong_release_and_unknown_scopes_fail_closed(self):
        with Host() as host, self.assertRaises(ContextError) as error:
            self.client(host, expected_incarnation='b' * 64)
        self.assertEqual(error.exception.code, 'stale')
        with Host(host_version='0.4.0') as host, self.assertRaises(ContextError) as error:
            self.client(host)
        self.assertEqual(error.exception.code, 'unsupported')
        with Host(scopes=('terminal_read', 'terminal_submit')) as host, self.assertRaises(ContextError):
            self.client(host)

    def test_nonprivate_or_symlink_endpoint_rejected_before_credential(self):
        with Host() as host:
            os.chmod(host.path, 0o666)
            with self.assertRaises(ContextError):
                self.client(host)
            self.assertEqual(host.frames, [])
        with tempfile.TemporaryDirectory(prefix='ryctx-') as directory:
            regular = Path(directory) / 'file'
            regular.write_text('not a socket')
            link = Path(directory) / 'link'
            link.symlink_to(regular)
            with self.assertRaises(ContextError):
                ContextClient(str(link), '1' * 64)

    def test_native_admission_error_preserves_typed_denial(self):
        with Host(auth_error={'type': 'registration_error', 'code': 'capability_denied', 'message': 'Pair this identity'}) as host:
            with self.assertRaises(ContextError) as error:
                self.client(host)
            self.assertEqual(error.exception.code, 'capability_denied')
            self.assertEqual(str(error.exception), 'Pair this identity')

    def test_primitive_mutation_success_is_uncertain_and_never_replayed(self):
        def reply(host, connection, request):
            host.send(connection, {'type': 'response', 'id': request['id'], 'result': True})
            return False
        scopes = ('terminal_read', 'terminal_propose')
        with Host(reply, scopes=scopes) as host, self.client(host, required_scopes=scopes, optional_scopes=()) as client:
            with self.assertRaises(ContextError) as error:
                client.request('terminal.input.propose', {'terminal': 't:1', 'text': 'hello'})
            self.assertEqual(error.exception.code, 'outcome_unknown')
            self.assertEqual(len(host.frames), 3)
            self.assertIsNone(client._socket)


class ContextSchemaTests(unittest.TestCase):
    def setUp(self):
        self.validator = Draft202012Validator(SCHEMA)

    def test_schema_and_every_request_shape_have_matching_closed_validation(self):
        Draft202012Validator.check_schema(SCHEMA)
        samples = {'buffer': 'b:1', 'pane': 'p:1', 'terminal': 't:1', 'snapshot': 's:1', 'proposal': 'i:1',
                   'expected_revision': 'r:1', 'from': 0, 'to': 1, 'offset': 0, 'limit': 10,
                   'max_rows': 200, 'max_bytes': 65536, 'max_cells': 65536, 'region': 'tail', 'text': 'cargo test',
                   'changes': [{'from': 0, 'to': 1, 'text': 'multi\nline'}]}
        for branch in SCHEMA['$defs']['request']['oneOf']:
            method = branch['properties']['method']['const']
            params = {key: samples[key] for key in branch['properties']['params']['required']}
            value = {'type': 'request', 'id': 'c:1', 'method': method, 'params': params}
            with self.subTest(method=method):
                self.assertTrue(self.validator.is_valid(value))
                validate_request(method, params)
                invalid = copy.deepcopy(value)
                invalid['params']['submit'] = False
                self.assertFalse(self.validator.is_valid(invalid))
                with self.assertRaises(ContextError):
                    validate_request(method, invalid['params'])

    def test_terminal_controls_and_structural_edit_errors_are_rejected(self):
        for text in ('\n', '\r', '\x1b[201~', '\x7f', '\x85', '\u2028', '\u2029', ''):
            value = {'type': 'request', 'id': 'c:1', 'method': 'terminal.input.propose', 'params': {'terminal': 't:1', 'text': text}}
            self.assertFalse(self.validator.is_valid(value))
            with self.assertRaises(ContextError):
                validate_request(value['method'], value['params'])
        for value in ({'offset': True, 'limit': 1}, {'offset': 0, 'limit': 0}, {'offset': 1 << 64, 'limit': 1}):
            with self.assertRaises(ContextError):
                validate_request('terminal.list', value)
        with self.assertRaises(ContextError):
            validate_request('buffer.edit', {'buffer': 'b:1', 'expected_revision': 'r:1',
                                            'changes': [{'from': 1, 'to': 3, 'text': 'x'}, {'from': 2, 'to': 3, 'text': 'y'}]})
        with self.assertRaises(ContextError):
            validate_request('terminal.input.propose', {'terminal': 't:1', 'text': '\ud800'})

    def test_ordinary_plugin_schema_does_not_admit_context_transport_messages(self):
        ordinary = json.loads((DIRECTORY / 'runyte-1.schema.json').read_text())
        validator = Draft202012Validator(ordinary)
        self.assertFalse(validator.is_valid({'type': 'authenticate', 'credential': 'a' * 64}))
        self.assertFalse(validator.is_valid({'type': 'request', 'id': 'c:1', 'method': 'terminal.read',
                                           'params': {'terminal': 't:1', 'region': 'tail', 'max_rows': 200, 'max_bytes': 65536, 'max_cells': 65536}}))


if __name__ == '__main__':
    unittest.main()
