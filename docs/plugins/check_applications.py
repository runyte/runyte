# SPDX-License-Identifier: MPL-2.0
"""Epoch 2 structural conformance; runtime preconditions are covered in Rust."""
import json
import selectors
import subprocess
import sys
from pathlib import Path
import unittest
import threading
import queue
from application import Application
from jsonschema import Draft202012Validator

DIRECTORY = Path(__file__).resolve().parent
SCHEMA = json.loads((DIRECTORY / 'runyte-experimental-2.schema.json').read_text())
FIXTURES = json.loads((DIRECTORY / 'epoch2-fixtures.json').read_text())

class ApplicationSchemaTests(unittest.TestCase):
    def test_cancellation_dispatch_survives_waiting_command_workers(self):
        app = Application('Test', [], [])
        requests = queue.Queue()
        cancelled = threading.Event()
        resource = threading.Event()
        app._write = lambda message: requests.put(message) if message['type'] == 'request' else None
        app.handlers['wait'] = lambda _: app.request('workspace.info') and None
        app.on_event = lambda name, data: cancelled.set() if name == 'job.cancel_requested' else None
        app.resource_handlers['resource.stat'] = lambda _: resource.set() or {'kind': 'stat', 'value': {}}
        try:
            for index in range(4):
                app._submit({'type': 'request', 'id': f'h:{index}', 'params': {'command': 'wait'}})
            for _ in range(4):
                requests.get(timeout=2)
            app._submit({'type': 'event', 'event': 'job.cancel_requested', 'data': {'job': 'j:1'}})
            self.assertTrue(cancelled.wait(1), 'Cancellation was starved by waiting commands')
            app._submit({'type': 'request', 'id': 'h:5', 'method': 'resource.stat', 'params': {}})
            self.assertTrue(resource.wait(1), 'Provider was starved by waiting commands')
        finally:
            with app._lock:
                for future in app._pending.values():
                    future.set_result({})
            app._resource_executor.shutdown(wait=True, cancel_futures=True)
            app._control.shutdown(wait=True, cancel_futures=True)
            app._executor.shutdown(wait=True, cancel_futures=True)

    def test_memory_provider_reads_version_bound_unicode_chunks(self):
        host = [f['message'] for f in FIXTURES if f['direction'] == 'host']
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'memory.py')],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)
        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()
        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(3), 'provider response timed out')
            message = json.loads(child.stdout.readline(1048577))
            Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]}).validate(message)
            return message
        def reply(request, result):
            send({'type': 'response', 'id': request['id'], 'result': result})
        try:
            send(host[0]); receive()
            send({**host[1], 'capabilities': ['providers', 'documents', 'jobs']})
            send({**host[2], 'params': {**host[2]['params'], 'arguments': {'key': 'alias'}}})
            register = receive()
            self.assertEqual(register['method'], 'provider.register')
            reply(register, {})
            opening = receive()
            self.assertEqual(opening['method'], 'resource.open')
            reply(opening, {'job': 'j:g:1', 'title': 'Open resource', 'state': 'running', 'progress': 0})
            self.assertEqual(receive()['result'], {'job': 'j:g:1'})
            send({'type': 'request', 'id': 'h:2', 'method': 'resource.stat',
                  'params': {'job': 'j:g:1', 'provider': 'memory', 'key': 'alias'}})
            metadata = receive()['result']['value']
            self.assertEqual(metadata['key'], 'notes')
            content = b''
            reads = 0
            while len(content) < metadata['bytes']:
                send({'type': 'request', 'id': f'h:{reads+3}', 'method': 'resource.read',
                      'params': {'job': 'j:g:1', 'provider': 'memory', 'key': 'notes',
                                 'version': metadata['version'], 'offset': len(content), 'limit': 131072}})
                chunk = receive()['result']['value']
                self.assertEqual(chunk['offset'], len(content))
                self.assertEqual(chunk['version'], metadata['version'])
                content += chunk['text'].encode()
                reads += 1
                self.assertEqual(chunk['eof'], len(content) == metadata['bytes'])
            self.assertGreater(reads, 1)
            self.assertIn('é猫 🦀\r\n'.encode(), content)
            def resource(method, number, **params):
                send({'type': 'request', 'id': f'h:{number}', 'method': method, 'params': params})
                return receive()
            started = resource('resource.write.begin', 100, job='j:g:save', provider='memory', key='notes',
                               expected_version=metadata['version'], bytes=5, encoding='utf-8')
            upload = started['result']['value']['upload']
            accepted = resource('resource.write.chunk', 101, job='j:g:save', upload=upload, offset=0, text='é猫')
            self.assertEqual(accepted['result']['value']['offset'], 5)
            committed = resource('resource.write.commit', 102, job='j:g:save', upload=upload, expected_version=metadata['version'])
            self.assertEqual(committed['result']['kind'], 'write_committed')
            current = resource('resource.stat', 103, job='j:g:read', provider='memory', key='notes')['result']['value']
            self.assertEqual(current['bytes'], 5)
            self.assertNotEqual(current['version'], metadata['version'])
            rejected = resource('resource.write.begin', 104, job='j:g:stale', provider='memory', key='notes',
                                expected_version=metadata['version'], bytes=0, encoding='utf-8')
            self.assertEqual(rejected['error']['code'], 'conflict')
            resource('resource.write.begin', 105, job='j:g:abort', provider='memory', key='notes',
                     expected_version=current['version'], bytes=0, encoding='utf-8')
            self.assertEqual(resource('resource.write.abort', 106, job='j:g:abort', upload=None)['result']['kind'], 'write_aborted')
            reconciled = resource('resource.reconcile', 107, job='j:g:rebind', provider='memory', key='notes', previous_write='j:g:save')
            self.assertEqual(reconciled['result']['value']['previous_write'], 'j:g:save')
            self.assertEqual(reconciled['result']['value']['metadata']['version'], current['version'])
            send({**host[2], 'id': 'h:108', 'params': {**host[2]['params'],
                  'command': 'inspect', 'context': 'buffer', 'buffer': 'b:g:1',
                  'buffer_revision': 'r:7', 'arguments': {}}})
            inspecting = receive()
            self.assertEqual(inspecting['method'], 'resource.inspect')
            self.assertEqual(inspecting['params'], {'buffer': 'b:g:1',
                             'expected_revision': 'r:7', 'invocation': 'h:108'})
            reply(inspecting, {'job': 'j:g:inspect', 'title': 'Inspect remote',
                              'state': 'running', 'progress': 0})
            self.assertEqual(receive(), {'type': 'response', 'id': 'h:108',
                                        'result': {'job': 'j:g:inspect'}})
        finally:
            child.stdin.close()
            child.wait(timeout=3)
            child.stdout.close()
            child.stderr.close()

    def test_file_manager_browses_then_requests_native_confirmation(self):
        host = [f['message'] for f in FIXTURES if f['direction'] == 'host']
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'files.py')],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)
        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()
        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(3), 'file manager response timed out')
            message = json.loads(child.stdout.readline(1048577))
            Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]}).validate(message)
            return message
        def reply(request, result):
            send({'type': 'response', 'id': request['id'], 'result': result})
        try:
            send(host[0])
            registration = receive()
            self.assertIn('filesystem', registration['required_capabilities'])
            send({**host[1], 'capabilities': ['views', 'filesystem', 'documents', 'interaction', 'jobs']})
            opening = {**host[2], 'params': {**host[2]['params'], 'arguments': {'path': '.'}}}
            send(opening)
            read = receive()
            self.assertEqual(read['method'], 'filesystem.list')
            self.assertEqual(read['params']['path'], '.')
            reply(read, {'directory': 'd:g:1', 'revision': 'd:1', 'next': None,
                         'entries': [{'entry': 'n:1', 'name': 'é.txt', 'kind': 'file', 'bytes': 5}]})
            create = receive()
            self.assertEqual(create['method'], 'view.create')
            self.assertEqual(create['params']['model']['rows'][0]['id'], 'n:1')
            reply(create, {'view': 'v:g:2', 'revision': 'm:1', 'model': create['params']['model']})
            show = receive()
            self.assertEqual(show['method'], 'pane.show')
            reply(show, {})
            self.assertEqual(receive()['id'], 'h:1')
            send({'type': 'request', 'id': 'h:2', 'method': 'command.invoke', 'params': {
                'command': 'rename', 'context': 'view', 'view': 'v:g:2', 'model_revision': 'm:1',
                'rows': ['n:1'], 'arguments': {'destination': 'new.txt'}}})
            prepared = receive()
            self.assertEqual(prepared['method'], 'filesystem.prepare')
            self.assertEqual(prepared['params'], {'directory': 'd:g:1', 'expected_revision': 'd:1',
                'intent': {'operation': 'rename', 'entry': 'n:1', 'destination': 'new.txt'}})
            reply(prepared, {'plan': 'f:g:3', 'operations': ['rename é.txt to new.txt']})
            confirmation = receive()
            self.assertEqual(confirmation['method'], 'filesystem.apply')
            self.assertEqual(confirmation['params'], {'plan': 'f:g:3', 'invocation': 'h:2'})
            reply(confirmation, {})
            self.assertEqual(receive(), {'type': 'response', 'id': 'h:2', 'result': {'job': None}})
            send({'type': 'request', 'id': 'h:3', 'method': 'command.invoke', 'params': {
                'command': 'new', 'context': 'view', 'view': 'v:g:2', 'model_revision': 'm:1',
                'rows': [], 'arguments': {}}})
            prompt = receive()
            self.assertEqual(prompt['method'], 'ui.prompt')
            reply(prompt, {'surface': 'u:g:4'})
            self.assertEqual(receive()['id'], 'h:3')
            send({'type': 'request', 'id': 'h:4', 'method': 'ui.submit', 'params': {
                'surface': 'u:g:4', 'accepted': True, 'values': {'destination': 'created.txt'}}})
            prepared = receive()
            self.assertEqual(prepared['params']['intent'], {'operation': 'create_file', 'destination': 'created.txt'})
            reply(prepared, {'plan': 'f:g:5', 'operations': ['create created.txt']})
            confirmation = receive()
            self.assertEqual(confirmation['params']['invocation'], 'h:4')
            reply(confirmation, {})
            self.assertEqual(receive()['id'], 'h:4')
            send({'type': 'request', 'id': 'h:5', 'method': 'command.invoke', 'params': {
                'command': 'enter', 'context': 'view', 'view': 'v:g:2', 'model_revision': 'm:1',
                'rows': ['n:1'], 'arguments': {}}})
            stat = receive()
            self.assertEqual(stat['method'], 'filesystem.stat')
            self.assertEqual(stat['params']['path'], 'é.txt')
            reply(stat, {'path': 'é.txt', 'kind': 'file', 'bytes': 5, 'revision': 's:1'})
            opening = receive()
            self.assertEqual(opening['method'], 'buffer.open')
            self.assertEqual(opening['params'], {'path': 'é.txt', 'invocation': 'h:5'})
            reply(opening, {'buffer': 'b:g:1', 'revision': 'r:0'})
            self.assertEqual(receive()['id'], 'h:5')

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

    def test_document_example_uses_captured_buffer_revision_and_returns_host_job(self):
        host = [f['message'] for f in FIXTURES if f['direction'] == 'host']
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'documents.py')],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)
        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()
        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(3), 'document example timed out')
            message = json.loads(child.stdout.readline(1048577))
            Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]}).validate(message)
            return message
        try:
            send(host[0])
            registration = receive()
            self.assertEqual(registration['required_capabilities'], ['documents', 'jobs'])
            send({**host[1], 'capabilities': ['documents', 'jobs']})
            for n, command in enumerate(['create', 'save', 'close'], 1):
                context = {**host[2]['params'], 'command': command,
                           'buffer': 'b:g:1', 'buffer_revision': 'r:7',
                           'arguments': {'path': 'notes.txt', 'text': 'é猫'} if command == 'create' else {}}
                send({'type': 'request', 'id': f'h:{n}', 'method': 'command.invoke', 'params': context})
                request = receive()
                self.assertEqual(request['method'], f'buffer.{command}')
                if command == 'create':
                    self.assertEqual(request['params'], {'path': 'notes.txt', 'text': 'é猫', 'invocation': f'h:{n}'})
                    result = {'buffer': 'b:g:1', 'revision': 'r:7', 'shown': True}
                else:
                    self.assertEqual(request['params'], {'buffer': 'b:g:1', 'expected_revision': 'r:7'})
                    result = {'job': 'j:g:2', 'state': 'running'} if command == 'save' else {}
                send({'type': 'response', 'id': request['id'], 'result': result})
                self.assertEqual(receive(), {'type': 'response', 'id': f'h:{n}',
                    'result': {'job': 'j:g:2' if command == 'save' else None}})
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

    def test_all_fixtures(self):
        Draft202012Validator.check_schema(SCHEMA)
        for fixture in FIXTURES:
            validator = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': f"#/$defs/{fixture['direction']}Message"}]})
            with self.subTest(message=fixture['message']):
                validator.validate(fixture['message'])
                malformed = {**fixture['message'], 'extra': True}
                if fixture['direction'] == 'plugin':
                    self.assertFalse(validator.is_valid(malformed))
                if malformed['type'] == 'response':
                    malformed['result'] = {'job': None}
                    malformed['error'] = {'code': 'internal', 'message': 'Failed'}
                    self.assertFalse(validator.is_valid(malformed))

    def test_remote_inspection_requires_explicit_foreground_invocation(self):
        fixture = next(f['message'] for f in FIXTURES if f['message'].get('method') == 'resource.inspect')
        validator = Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]})
        for invocation in [None, 'missing']:
            params = {**fixture['params'], 'invocation': invocation}
            if invocation == 'missing':
                del params['invocation']
            self.assertFalse(validator.is_valid({**fixture, 'params': params}))

    def test_task_application_runs_through_public_messages(self):
        host = [f['message'] for f in FIXTURES if f['direction'] == 'host']
        child = subprocess.Popen([sys.executable, str(DIRECTORY / 'tasks.py')],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0)
        def send(message):
            child.stdin.write((json.dumps(message) + '\n').encode())
            child.stdin.flush()
        def receive():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(3), 'application response timed out')
            data = child.stdout.readline(1048577)
            self.assertTrue(data.endswith(b'\n'))
            message = json.loads(data)
            Draft202012Validator({**SCHEMA, 'anyOf': [{'$ref': '#/$defs/pluginMessage'}]}).validate(message)
            return message
        try:
            send(host[0])
            self.assertEqual(receive()['type'], 'register')
            send({**host[1], 'capabilities': ['views']})
            send(host[2])
            create = receive()
            self.assertEqual(create['method'], 'view.create')
            model = create['params']['model']
            send({'type': 'response', 'id': create['id'], 'result': {'view': 'v:g:1', 'revision': 'm:1', 'model': model}})
            show = receive()
            self.assertEqual(show['params'], {'invocation': 'h:1', 'view': 'v:g:1'})
            send({'type': 'response', 'id': show['id'], 'result': {}})
            self.assertEqual(receive(), {'type': 'response', 'id': 'h:1', 'result': {'job': None}})
            send({'type': 'request', 'id': 'h:2', 'method': 'command.invoke',
                  'params': {'command': 'toggle', 'context': 'view', 'view': 'v:g:1', 'model_revision': 'm:1', 'rows': ['read'], 'arguments': {}}})
            publish = receive()
            self.assertEqual(publish['method'], 'view.publish')
            model = publish['params']['model']
            self.assertTrue(model['rows'][0]['text'].startswith('[x]'))
            send({'type': 'response', 'id': publish['id'], 'result': {'view': 'v:g:1', 'revision': 'm:2', 'model': model}})
            self.assertEqual(receive()['id'], 'h:2')
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
