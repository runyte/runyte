# SPDX-License-Identifier: MPL-2.0
"""Public handoff/feedback shapes and SDK non-replay guarantees; no browser launch."""
import importlib.util
import json
from pathlib import Path
import unittest
from application import Application, PluginError
from check_queries import shutdown


class HandoffSdkTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Handoff test', [], ['terminals', 'external', 'notifications'])
        self.calls = []
        def request(method, **params):
            self.calls.append((method, params))
            return {}
        self.app.request = request

    def tearDown(self):
        shutdown(self.app)

    def test_terminal_preserves_literal_argument_boundaries_and_explicit_context(self):
        self.app.open_terminal('h:1', 'Tools', '/usr/bin/tool', ['é猫', 'a b', '$(literal)'])
        self.assertEqual(self.calls, [('terminal.open', {'invocation': 'h:1', 'label': 'Tools',
                         'executable': '/usr/bin/tool', 'args': ['é猫', 'a b', '$(literal)']})])
        self.app.open_terminal('h:2', 'Tools', 'tool', cwd='subdirectory')
        self.assertEqual(self.calls[-1][1]['cwd'], 'subdirectory')

    def test_external_target_kinds_are_explicit_and_never_replayed(self):
        self.app.open_url('h:1', 'https://example.org/path?q=a%20b')
        self.app.open_file_externally('h:2', 'report with spaces.pdf')
        self.assertEqual(self.calls, [
            ('external.open', {'invocation': 'h:1', 'target': {'kind': 'url', 'url': 'https://example.org/path?q=a%20b'}}),
            ('external.open', {'invocation': 'h:2', 'target': {'kind': 'file', 'path': 'report with spaces.pdf'}})])
        attempts = []
        def fail(method, **params):
            attempts.append((method, params))
            raise PluginError('outcome_unknown', 'Handler startup outcome is unknown')
        self.app.request = fail
        with self.assertRaises(PluginError):
            self.app.open_url('h:3', 'https://example.org/')
        self.assertEqual(len(attempts), 1)

    def test_notification_has_no_caller_chosen_owner_or_foreground_token(self):
        self.app.publish_notification('info', 'Title')
        self.app.publish_notification('error', 'Failure', 'Inspect before retrying.\nDetails')
        self.assertEqual(self.calls, [
            ('notification.publish', {'severity': 'info', 'title': 'Title', 'body': ''}),
            ('notification.publish', {'severity': 'error', 'title': 'Failure', 'body': 'Inspect before retrying.\nDetails'})])

    def test_handoff_and_notification_fixtures_match_the_published_schema(self):
        from jsonschema import Draft202012Validator
        directory = Path(__file__).parent
        schema = json.loads((directory / 'runyte-experimental-2.schema.json').read_text())
        fixtures = json.loads((directory / 'epoch2-fixtures.json').read_text())
        checked = 0
        for fixture in fixtures:
            if fixture['message'].get('id') in {f'p:{i}' for i in range(1100, 1104)}:
                validator = Draft202012Validator({'$defs': schema['$defs'], '$ref': f'#/$defs/{fixture["direction"]}Message'})
                self.assertEqual(list(validator.iter_errors(fixture['message'])), [])
                checked += 1
        self.assertEqual(checked, 5)
        validator = Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/notification.publish'})
        spoof = {'type': 'request', 'id': 'p:1', 'method': 'notification.publish',
                 'params': {'severity': 'error', 'title': 'Title', 'body': '', 'source': 'Runyte'}}
        self.assertFalse(validator.is_valid(spoof))

    def test_example_routes_only_the_explicit_command_to_its_matching_handoff(self):
        spec = importlib.util.spec_from_file_location('handoff_example_test', Path(__file__).with_name('handoffs.py'))
        example = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(example)
        example.app.request = self.app.request
        try:
            example.browser({'invocation': 'h:4', 'arguments': {'url': 'https://example.org/'}})
            example.file({'invocation': 'h:5', 'arguments': {'path': 'report.pdf'}})
            example.notify({'invocation': 'h:6'})
            self.assertEqual([method for method, _ in self.calls], ['external.open', 'external.open', 'notification.publish'])
            self.assertEqual(self.calls[0][1]['invocation'], 'h:4')
            self.assertEqual(self.calls[1][1]['invocation'], 'h:5')
        finally:
            shutdown(example.app)


if __name__ == '__main__':
    unittest.main()
