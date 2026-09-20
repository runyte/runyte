# SPDX-License-Identifier: MPL-2.0
"""Shared release vectors and stable client negotiation; standard library only."""
import copy
import json
from pathlib import Path
import unittest

from application import Application, PluginError, ReleaseRange, VERSION, _version


TEST_LIMITS = next(f['message']['limits'] for f in json.loads(
    Path(__file__).with_name('stable-fixtures.json').read_text())
    if f['message']['type'] == 'hello')

class CompatibilityTests(unittest.TestCase):
    def test_shared_release_vectors(self):
        vectors = json.loads((Path(__file__).parent / 'compatibility/ranges.json').read_text())
        for case in vectors['ranges']:
            with self.subTest(case=case['range']):
                if not case['valid']:
                    with self.assertRaises(ValueError):
                        ReleaseRange(case['range'])
                    continue
                value = ReleaseRange(case['range'])
                for key, expected in [('accept', True), ('reject', False)]:
                    for host in case[key]:
                        self.assertEqual(value.contains(host), expected, host)
                if 'normalized' in case:
                    self.assertEqual(value.normalized, case['normalized'])
        for case in vectors['subsets']:
            self.assertEqual(ReleaseRange(case['configured']).is_subset_of(ReleaseRange(case['authored'])), case['expected'])
        for version in vectors['invalid_versions']:
            with self.subTest(version=version), self.assertRaises(ValueError):
                _version(version)

    def handshake(self, *, hello=None, registered=None, commands=(), **kwargs):
        app = Application('Check', list(commands), ['views'], runyte='>=0.3.0, <0.4.0', **kwargs)
        messages = iter([
            hello if hello is not None else {'type': 'hello', 'version': VERSION, 'host_version': '0.3.1', 'capabilities': ['views', 'jobs'], 'features': ['extension'], 'limits': TEST_LIMITS, 'future': True},
            registered if registered is not None else {'type': 'registered', 'runyte': '>=0.3.1, <0.4.0', 'capabilities': ['views'], 'features': [], 'limits': TEST_LIMITS, 'future': True},
        ])
        def read():
            try:
                return next(messages)
            except StopIteration:
                raise EOFError from None
        self.last_app = app
        app._read = read
        sent = []
        self.last_sent = sent
        app._write = sent.append
        app.run()
        return app, sent

    def test_presentation_handshake_omits_unsupported_fields_and_requires_ack(self):
        commands = [{'name': 'open', 'description': 'Open', 'context': 'view',
                     'presentation': {'label': 'Open value'}}]
        app, sent = self.handshake(commands=commands, optional_features=['view-action-presentation'])
        self.assertNotIn('presentation', sent[0]['commands'][0])
        self.assertIn('presentation', app.commands[0])
        hello = {'type': 'hello', 'version': VERSION, 'host_version': '0.3.1',
                 'capabilities': ['views'], 'features': ['view-action-presentation'], 'limits': TEST_LIMITS}
        with self.assertRaises(PluginError) as error:
            self.handshake(hello=hello, commands=commands, optional_features=['view-action-presentation'])
        self.assertEqual(error.exception.code, 'invalid_registration')
        ack = {'type': 'registered', 'runyte': '>=0.3.1, <0.4.0', 'capabilities': ['views'],
               'features': ['view-action-presentation'], 'limits': TEST_LIMITS}
        _, sent = self.handshake(hello=hello, registered=ack, commands=commands,
                                 optional_features=['view-action-presentation'])
        self.assertEqual(sent[0]['commands'][0]['presentation'], {'label': 'Open value'})

    def test_acknowledged_state_and_ignorable_additions(self):
        app, sent = self.handshake(optional_features=['extension', 'unavailable'], optional_capabilities=['jobs'])
        self.assertEqual(app.host_version, '0.3.1')
        self.assertEqual(app.granted_capabilities, {'views'})
        self.assertEqual(app.features, set())
        self.assertEqual(sent[0]['runyte'], '>=0.3.0, <0.4.0')
        self.assertEqual(sent[0]['optional_features'], ['extension', 'unavailable'])

    def test_row_actions_optional_feature_keeps_legacy_clients_unchanged(self):
        hello = {'type': 'hello', 'version': VERSION, 'host_version': '0.3.0',
                 'capabilities': ['views'], 'features': ['view-row-actions'], 'limits': TEST_LIMITS}
        ack = {'type': 'registered', 'runyte': '>=0.3.0, <0.4.0',
               'capabilities': ['views'], 'features': [], 'limits': TEST_LIMITS}
        app, _ = self.handshake(hello=hello, registered=ack)
        self.assertEqual(app.features, set())
        app, _ = self.handshake(hello=hello, registered={**ack, 'features': ['view-row-actions']},
                                optional_features=['view-row-actions'])
        self.assertEqual(app.features, {'view-row-actions'})
        app, _ = self.handshake(hello={**hello, 'features': []}, registered=ack,
                                optional_features=['view-row-actions'])
        self.assertEqual(app.features, set())

    def test_invalid_or_unsupported_host_never_receives_registration(self):
        for hello in [[], {'type': 'hello', 'version': 'runyte-experimental-2'},
                      {'type': 'hello', 'version': VERSION, 'host_version': '0.2.9'},
                      {'type': 'hello', 'version': VERSION, 'host_version': 'bad'}]:
            with self.subTest(hello=hello), self.assertRaises(PluginError):
                self.handshake(hello=hello)

    def test_required_features_and_invalid_acknowledgements(self):
        with self.assertRaises(PluginError):
            self.handshake(required_features=['unavailable'])
        base = {'type': 'registered', 'runyte': '>=0.3.0, <0.4.0', 'capabilities': ['views'], 'features': [], 'limits': TEST_LIMITS}
        for ack in [[], {**base, 'capabilities': []}, {**base, 'capabilities': ['views', 'jobs']},
                    {**base, 'features': ['extension']}, {**base, 'runyte': '>=0.2.0, <0.4.0'},
                    {**base, 'runyte': '=0.3.0'}, {**base, 'features': ['x', 'x']}]:
            with self.subTest(ack=ack), self.assertRaises(PluginError):
                self.handshake(registered=ack)

    def test_limits_are_validated_before_registration_or_publishing_negotiated_state(self):
        hello = {'type': 'hello', 'version': VERSION, 'host_version': '0.3.1',
                 'capabilities': ['views'], 'features': [], 'limits': TEST_LIMITS}
        registered = {'type': 'registered', 'runyte': '>=0.3.0, <0.4.0',
                      'capabilities': ['views'], 'features': [], 'limits': TEST_LIMITS}
        malformed = [None, [], {}, {'line_bytes': 'bad', 'requests': False}]
        for name in TEST_LIMITS:
            if name == 'resources':
                continue
            malformed.append({key: value for key, value in TEST_LIMITS.items() if key != name})
            malformed.append({**TEST_LIMITS, name: TEST_LIMITS[name] - 1})
        for value in (False, True, 0, -1, 1.5, '1048576', (1 << 64)):
            malformed.append({**TEST_LIMITS, 'line_bytes': value})
        malformed.extend([{**TEST_LIMITS, 'resources': value} for value in (None, [], {})])
        for name in TEST_LIMITS['resources']:
            malformed.append({**TEST_LIMITS, 'resources': {
                **TEST_LIMITS['resources'], name: TEST_LIMITS['resources'][name] - 1}})
        for phase, base in [('hello', hello), ('registered', registered)]:
            for index, limits in enumerate(malformed):
                with self.subTest(phase=phase, case=index):
                    with self.assertRaises(PluginError) as caught:
                        self.handshake(**{phase: {**base, 'limits': limits}})
                    self.assertEqual(caught.exception.code, 'invalid_registration')
                    self.assertIsNone(self.last_app.host_version)
                    self.assertEqual(self.last_app.granted_capabilities, set())
                    self.assertEqual(self.last_app.features, set())
                    self.assertEqual(self.last_app.limits, {})
                    self.assertEqual(len(self.last_sent), 0 if phase == 'hello' else 1)
            with self.subTest(phase=phase, missing=True), self.assertRaises(PluginError):
                self.handshake(**{phase: {key: value for key, value in base.items() if key != 'limits'}})

    def test_limits_allow_optional_resource_inventory_and_unknown_additions(self):
        limits = copy.deepcopy(TEST_LIMITS)
        limits['requests'] += 1
        limits['future'] = {'informational': True}
        limits['resources']['future'] = 'informational'
        for inventory in (limits, {key: value for key, value in limits.items() if key != 'resources'}):
            hello = {'type': 'hello', 'version': VERSION, 'host_version': '0.3.1',
                     'capabilities': ['views'], 'features': [], 'limits': inventory}
            registered = {'type': 'registered', 'runyte': '>=0.3.0, <0.4.0',
                          'capabilities': ['views'], 'features': [], 'limits': inventory}
            app, _ = self.handshake(hello=hello, registered=registered)
            self.assertEqual(app.limits, inventory)
            self.assertEqual(app.granted_capabilities, {'views'})

    def test_host_rejection_retains_reason_code(self):
        with self.assertRaises(PluginError) as caught:
            self.handshake(registered={'type': 'registration_error', 'code': 'range_conflict', 'message': 'Regenerate configuration'})
        self.assertEqual(caught.exception.code, 'range_conflict')


if __name__ == '__main__':
    unittest.main(verbosity=2)
