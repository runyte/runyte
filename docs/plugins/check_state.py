# SPDX-License-Identifier: MPL-2.0
"""Explicit settings/schema registration and non-replaying state ownership."""
import importlib.util
import concurrent.futures
import json
import unittest
from pathlib import Path
from unittest.mock import patch
from application import Application, PluginError, STATE_LIMIT, VERSION
from check_queries import shutdown


class StateSdkTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Preferences', [], ['settings', 'state'])
        self.calls = []
        self.result = {'revision': 's:missing', 'document': None}
        def request(method, **params):
            self.calls.append((method, params))
            return self.result
        self.app.request = request

    def tearDown(self):
        shutdown(self.app)

    def test_settings_and_state_keep_distinct_ownership_and_explicit_version(self):
        self.result = {'settings': {'default_destination': 'é猫'}}
        self.assertEqual(self.app.get_settings(), {'default_destination': 'é猫'})
        self.result = {'revision': 's:missing', 'document': None}
        self.assertEqual(self.app.get_state(), self.result)
        self.app.set_state('s:missing', 3, {'destination': 'é猫'})
        self.app.delete_state('s:' + 'a' * 64)
        self.assertEqual(self.calls, [
            ('settings.get', {}), ('state.get', {}),
            ('state.set', {'expected_revision': 's:missing',
                           'document': {'version': 3, 'data': {'destination': 'é猫'}}}),
            ('state.delete', {'expected_revision': 's:' + 'a' * 64})])

    def test_conflict_and_unknown_outcome_never_trigger_replay_or_migration(self):
        for code in ('conflict', 'outcome_unknown', 'busy'):
            for delete in (False, True):
                self.calls.clear()
                def fail(method, **params):
                    self.calls.append((method, params))
                    raise PluginError(code, 'State operation did not complete normally')
                self.app.request = fail
                with self.assertRaises(PluginError) as caught:
                    if delete:
                        self.app.delete_state('s:missing')
                    else:
                        self.app.set_state('s:missing', 1, {})
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(len(self.calls), 1)

    def test_invalid_versions_and_nonfinite_or_oversized_json_never_go_on_wire(self):
        for version in (-1, 0x100000000, True, '1', 1.5):
            with self.assertRaises(PluginError):
                self.app.set_state('s:missing', version, {})
        cyclic = []
        cyclic.append(cyclic)
        for data in (float('nan'), float('inf'), b'not JSON', cyclic,
                     '猫' * (STATE_LIMIT // 3 + 1)):
            with self.assertRaises(PluginError):
                self.app.set_state('s:missing', 1, data)
        self.assertEqual(self.calls, [])

    def test_lost_mutation_reply_is_unknown_with_host_deadline_delivery_headroom(self):
        waits = []
        sent = []
        class TimedOutFuture(concurrent.futures.Future):
            def result(self, timeout=None):
                waits.append(timeout)
                raise concurrent.futures.TimeoutError()
        self.app.request = Application.request.__get__(self.app, Application)
        self.app._write = sent.append
        with patch('application.concurrent.futures.Future', TimedOutFuture):
            for operation, code in ((lambda: self.app.get_state(), 'timeout'),
                                    (lambda: self.app.set_state('s:missing', 1, {}), 'outcome_unknown'),
                                    (lambda: self.app.delete_state('s:missing'), 'outcome_unknown')):
                with self.assertRaises(PluginError) as caught:
                    operation()
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(self.app._pending, {})
        self.assertEqual(len(waits), 3)
        self.assertTrue(all(0 < remaining <= 12 for remaining in waits))
        self.assertEqual([message['method'] for message in sent], ['state.get', 'state.set', 'state.delete'])

    def test_registration_includes_only_explicit_settings_schema(self):
        for schema in (None, {'fields': [{'name': 'limit', 'type': 'integer', 'min': 1, 'max': 20}]}):
            app = Application('Settings schema', [], ['settings'], settings_schema=schema)
            messages = iter([{'type': 'hello', 'version': VERSION}, {'type': 'registered'}])
            def read():
                try:
                    return next(messages)
                except StopIteration:
                    raise EOFError() from None
            sent = []
            app._read = read
            app._write = sent.append
            app.run()
            self.assertEqual(len(sent), 1)
            if schema is None:
                self.assertNotIn('settings_schema', sent[0])
            else:
                self.assertEqual(sent[0]['settings_schema'], schema)

    def test_public_settings_state_shapes_and_schema_constraints(self):
        from jsonschema import Draft202012Validator
        directory = Path(__file__).parent
        schema = json.loads((directory / 'runyte-experimental-2.schema.json').read_text())
        fixtures = json.loads((directory / 'epoch2-fixtures.json').read_text())
        checked = 0
        for fixture in fixtures:
            if fixture['message'].get('id') in {f'p:{i}' for i in range(1300, 1304)}:
                validator = Draft202012Validator({'$defs': schema['$defs'],
                                                  '$ref': f'#/$defs/{fixture["direction"]}Message'})
                self.assertEqual(list(validator.iter_errors(fixture['message'])), [], fixture)
                checked += 1
        self.assertEqual(checked, 8)
        validator = Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/settingsSchema'})
        self.assertTrue(validator.is_valid({'fields': [{'name': 'title', 'type': 'string', 'enum': None}]}))
        for field in ({'name': 'x', 'type': 'number'}, {'name': 'x', 'type': 'boolean', 'min': 1},
                      {'name': 'x', 'type': 'string', 'enum': []},
                      {'name': 'x', 'type': 'string', 'enum': ['a', 'a']},
                      {'name': 'x', 'type': 'integer', 'required': None}):
            self.assertFalse(validator.is_valid({'fields': [field]}), field)

    def test_reference_example_refuses_implicit_migration_and_does_not_replay_conflict(self):
        spec = importlib.util.spec_from_file_location('preferences_test', Path(__file__).with_name('preferences.py'))
        example = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(example)
        calls = []
        document = {'version': 2, 'data': {'destination': 'preserve'}}
        def request(method, **params):
            calls.append((method, params))
            if method == 'state.get':
                return {'revision': 's:' + 'b' * 64, 'document': document}
            if method == 'settings.get':
                return {'settings': {'default_destination': '.'}}
            raise PluginError('conflict', 'State changed')
        example.app.request = request
        try:
            context = {'arguments': {'destination': 'sample'}}
            with self.assertRaises(PluginError) as caught:
                example.remember(context)
            self.assertEqual(caught.exception.code, 'unsupported')
            self.assertFalse(any(method == 'state.set' for method, _ in calls))
            document['version'] = 1
            calls.clear()
            with self.assertRaises(PluginError) as caught:
                example.remember(context)
            self.assertEqual(caught.exception.code, 'conflict')
            self.assertEqual([method for method, _ in calls], ['state.get', 'settings.get', 'state.set'])
            self.assertEqual(calls[-1][1]['expected_revision'], 's:' + 'b' * 64)
        finally:
            shutdown(example.app)


class PreferencesReviewTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('preferences_review', Path(__file__).with_name('preferences.py'))
        self.example = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.example)

    def tearDown(self):
        shutdown(self.example.app)

    def test_state_set_freezes_the_validated_document_before_request_admission(self):
        data = {'nested': ['original']}
        captured = []
        def request(method, **params):
            data['nested'][0] = float('nan')
            captured.append((method, params))
            return {'revision': 's:' + 'a' * 64, 'document': None}
        self.example.app.request = request
        self.example.app.set_state('s:missing', 1, data)
        self.assertEqual(captured[0][1]['document'], {'version': 1, 'data': {'nested': ['original']}})

    def test_preferences_quotes_control_characters_without_changing_destination_identity(self):
        current = 'folder\u0085name\u2028雪'
        models = []
        def request(method, **params):
            if method == 'settings.get':
                return {'settings': {'default_destination': current}}
            if method == 'state.get':
                return {'revision': 's:missing', 'document': None}
            if method == 'view.create':
                models.append(params['model'])
                return {'view': 'v:1', 'revision': 'm:1'}
            if method == 'pane.show':
                return {}
            self.fail(method)
        self.example.app.request = request
        self.example.open_view({'invocation': 'h:1'})
        rendered = models[0]['rows'][0]['text']
        self.assertTrue(all(character.isprintable() for character in rendered))
        self.assertEqual(json.loads(rendered.removeprefix('Destination: ')), current)

    def test_newer_open_and_closed_view_invalidate_an_older_pending_read(self):
        import threading
        for close in (False, True):
            with self.subTest(close=close):
                waiting, release = threading.Event(), threading.Event()
                models, shown, errors = [], [], []
                if close:
                    self.example.view, self.example.revision = 'v:old', 'm:1'
                else:
                    self.example.view = self.example.revision = None
                def request(method, **params):
                    if method == 'settings.get':
                        return {'settings': {}}
                    if method == 'state.get':
                        old = threading.current_thread().name == 'old-open'
                        if old:
                            waiting.set()
                            self.assertTrue(release.wait(2))
                        return {'revision': 's:' + 'a' * 64, 'document': {
                            'version': 1, 'data': {'destination': 'old' if old else 'new'}}}
                    if method in ('view.create', 'view.publish'):
                        models.append(params['model'])
                        return {'view': 'v:new', 'revision': 'm:2'}
                    if method == 'pane.show':
                        shown.append(params['invocation'])
                        return {}
                    self.fail(method)
                self.example.app.request = request
                def old_open():
                    try:
                        self.example.open_view({'invocation': 'h:old'})
                    except Exception as error:
                        errors.append(error)
                worker = threading.Thread(target=old_open, name='old-open')
                worker.start()
                try:
                    self.assertTrue(waiting.wait(2))
                    if close:
                        self.example.event('view.closed', {'view': 'v:old'})
                    else:
                        self.example.open_view({'invocation': 'h:new'})
                finally:
                    release.set()
                    worker.join(2)
                self.assertFalse(worker.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], PluginError)
                self.assertEqual(errors[0].code, 'context_changed')
                self.assertEqual(shown, [] if close else ['h:new'])
                self.assertEqual(len(models), 0 if close else 1)
                if models:
                    self.assertEqual(models[0]['rows'][0]['text'], 'Destination: "new"')


class StateDisconnectTests(unittest.TestCase):
    def test_local_eof_after_send_distinguishes_mutations_from_reads(self):
        import threading
        for method in ('state.get', 'state.set', 'state.delete'):
            with self.subTest(method=method):
                app = Application('State disconnect', [], ['state'])
                registered, sent = threading.Event(), threading.Event()
                frames, errors = [], []
                incoming = iter([{'type': 'hello', 'version': VERSION}, {'type': 'registered'}])
                def read():
                    try:
                        message = next(incoming)
                        if message['type'] == 'registered':
                            registered.set()
                        return message
                    except StopIteration:
                        self.assertTrue(sent.wait(2))
                        raise EOFError() from None
                def write(message):
                    frames.append(message)
                    if message['type'] == 'request':
                        sent.set()
                def request():
                    try:
                        self.assertTrue(registered.wait(2))
                        app.request(method)
                    except Exception as error:
                        errors.append(error)
                app._read, app._write = read, write
                worker = threading.Thread(target=request)
                worker.start()
                try:
                    app.run()
                finally:
                    registered.set()
                    worker.join(2)
                    shutdown(app)
                self.assertFalse(worker.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], PluginError)
                self.assertEqual(errors[0].code, 'unavailable' if method == 'state.get' else 'outcome_unknown')
                self.assertEqual(len([frame for frame in frames if frame['type'] == 'request']), 1)
                self.assertEqual(app._pending, {})

    def test_known_refusals_stay_known_and_failed_pipe_mutations_are_uncertain(self):
        for method in ('state.set', 'state.delete'):
            for failure in ('closed', 'host', 'pipe', 'limit'):
                with self.subTest(method=method, failure=failure):
                    app = Application('State failure', [], ['state'])
                    calls = []
                    def write(message):
                        calls.append(message)
                        if failure == 'pipe':
                            raise BrokenPipeError('private path must not appear')
                        if failure == 'limit':
                            raise PluginError('limit_exceeded', 'Encoded message exceeds limit')
                        app._response({'id': message['id'], 'error': {
                            'code': 'unavailable', 'message': 'State unavailable'}})
                    app._write = write
                    if failure == 'closed':
                        app._closed.set()
                    try:
                        with self.assertRaises(PluginError) as caught:
                            app.request(method)
                        self.assertEqual(caught.exception.code, {
                            'closed': 'unavailable', 'host': 'unavailable',
                            'pipe': 'outcome_unknown', 'limit': 'limit_exceeded'}[failure])
                        self.assertNotIn('private path', str(caught.exception))
                        self.assertEqual(len(calls), 0 if failure == 'closed' else 1)
                        self.assertEqual(app._pending, {})
                    finally:
                        shutdown(app)


class StateBoundsTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('State bounds', [], ['state'])
        self.calls = []
        self.app.request = lambda method, **params: self.calls.append((method, params))

    def tearDown(self):
        shutdown(self.app)

    def test_identifiers_preserve_exact_integer_boundaries_and_larger_strings(self):
        values = {'minimum': -(1 << 63), 'maximum': (1 << 64) - 1,
                  'above_signed': 1 << 63, 'as_string': str(1 << 100),
                  'bool': True, 'float': 1.25e100}
        self.app.set_state('s:missing', 0xffffffff, values)
        self.assertEqual(self.calls[0][1]['document']['data'], values)
        self.assertIs(type(self.calls[0][1]['document']['data']['maximum']), int)
        self.assertIs(type(self.calls[0][1]['document']['data']['bool']), bool)
        self.calls.clear()
        for value in (-(1 << 63) - 1, 1 << 64, 123456789012345678901234567890):
            with self.assertRaises(PluginError) as caught:
                self.app.set_state('s:missing', 1, {'identifier': value})
            self.assertEqual(caught.exception.code, 'invalid_argument')
            self.assertNotIn(str(value), str(caught.exception))
        self.assertEqual(self.calls, [])

    def test_whole_document_depth_and_container_boundaries_precede_encoding(self):
        nested = 0
        for _ in range(14):
            nested = [nested]
        self.app.set_state('s:missing', 1, nested)  # Envelope depth1, data depth2.
        self.app.set_state('s:missing', 1, [0] * 1024)
        self.app.set_state('s:missing', 1, {str(i): i for i in range(1024)})
        self.calls.clear()
        cyclic = []; cyclic.append(cyclic)
        for value in ([nested], [0] * 1025, {str(i): i for i in range(1025)}, cyclic):
            with patch('application.json.dumps', side_effect=AssertionError('must reject before encoding')):
                with self.assertRaises(PluginError) as caught:
                    self.app.set_state('s:missing', 1, value)
            self.assertEqual(caught.exception.code, 'limit_exceeded')
        self.assertEqual(self.calls, [])

    def test_whole_document_node_boundary_counts_envelope_and_container_nodes(self):
        # 1 envelope + version scalar + data array + 16 arrays + 16,365 scalars.
        values = [[0] * 1023 for _ in range(15)] + [[0] * 1020]
        self.app.set_state('s:missing', 1, values)
        self.assertEqual(len(self.calls), 1)
        self.calls.clear()
        values[-1].append(0)
        with patch('application.json.dumps', side_effect=AssertionError('must reject before encoding')):
            with self.assertRaises(PluginError) as caught:
                self.app.set_state('s:missing', 1, values)
        self.assertEqual(caught.exception.code, 'limit_exceeded')
        self.assertEqual(self.calls, [])

    def test_non_string_keys_and_non_json_values_are_not_coerced(self):
        for value in ({1: 'secret'}, {False: 'secret'}, {None: 'secret'}, (1, 2), {'nested': float('inf')}):
            with self.assertRaises(PluginError) as caught:
                self.app.set_state('s:missing', 1, value)
            self.assertEqual(caught.exception.code, 'invalid_argument')
            self.assertNotIn('secret', str(caught.exception))
        self.assertEqual(self.calls, [])


if __name__ == '__main__':
    unittest.main()
