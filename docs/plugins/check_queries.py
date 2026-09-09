# SPDX-License-Identifier: MPL-2.0
"""Query preconditions, reliable observation routing and bounded catalog lookup."""
import importlib.util
import json
import queue
import unittest
from pathlib import Path
from unittest.mock import patch
from application import Application, PluginError
from check_models import ModelPort


def shutdown(app):
    for executor in (app._executor, app._resource_executor, app._control,
                     app._observations, app._validation_executor):
        executor.shutdown(wait=True, cancel_futures=True)


class QuerySdkTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Queries', [], ['views'])
        self.large = {'title': 'Large', 'purpose': 'list', 'rows': [
            {'id': 'row', 'text': 'é猫' * 300000, 'role': 'ordinary'}]}
        self.port = ModelPort(self.large)
        self.app.request = self.port.request

    def tearDown(self):
        shutdown(self.app)

    def test_query_enable_and_retry_forward_exact_preconditions(self):
        self.app.set_query('v:1', 'm:1', '')
        self.app.set_query('v:1', 'm:1', 'é猫', expected_query_revision='qv:1')
        self.assertEqual(self.port.calls, [
            ('view.query.set', {'view': 'v:1', 'expected_revision': 'm:1', 'text': ''}),
            ('view.query.set', {'view': 'v:1', 'expected_revision': 'm:1', 'text': 'é猫',
                                'expected_query_revision': 'qv:1'})])

    def test_inline_model_and_patch_bind_query_without_changing_legacy_calls(self):
        model = {**self.large, 'rows': []}
        self.app.publish_model('v:1', 'm:1', model, expected_query_revision='qv:2')
        self.app.patch_view('v:1', 'm:1', [], expected_query_revision='qv:2')
        self.app.publish_model('v:1', 'm:1', model)
        self.assertEqual([params.get('expected_query_revision') for _, params in self.port.calls],
                         ['qv:2', 'qv:2', None])
        self.assertNotIn('expected_query_revision', self.port.calls[-1][1])

    def test_staged_model_and_patch_bind_token_at_open(self):
        for patch_model in (False, True):
            self.port.calls.clear(); self.port.staged.clear()
            if patch_model:
                self.app.patch_view('v:1', 'm:1', [{'kind': 'update', 'row': self.large['rows'][0]}],
                                    expected_query_revision='qv:7')
            else:
                self.app.publish_model('v:1', 'm:1', self.large, expected_query_revision='qv:7')
            self.assertEqual(self.port.calls[0][0], 'view.stage.open')
            self.assertEqual(self.port.calls[0][1]['expected_query_revision'], 'qv:7')
            commit = [params for method, params in self.port.calls if method == 'view.stage.commit']
            self.assertEqual(commit, [{'stage': 's:1'}])

    def test_stale_staged_query_is_closed_and_never_retried(self):
        self.port.fault = ('view.stage.commit', 'stale')
        with self.assertRaises(PluginError):
            self.app.publish_model('v:1', 'm:1', self.large, expected_query_revision='qv:1')
        methods = [method for method, _ in self.port.calls]
        self.assertEqual(methods.count('view.stage.commit'), 1)
        self.assertEqual(methods[-1], 'view.stage.close')
        self.assertNotIn('view.query.set', methods)

    def test_large_read_retains_query_metadata_from_initial_get(self):
        original = self.port.request
        captured = {'revision': 'qv:3', 'text': 'é猫', 'pending': True}
        def request(method, **params):
            result = original(method, **params)
            if method == 'view.get':
                result['query'] = captured
            return result
        self.app.request = request
        result = self.app.get_model('v:1')
        self.assertEqual(result['model'], self.large)
        self.assertEqual(result['query'], captured)

    def test_action_observations_use_ordered_lane_without_invoking_commands(self):
        received = queue.Queue()
        self.app._subscriptions['o:1'] = lambda event, sequence, data: received.put((event, sequence, data))
        for serial in range(2):
            self.app._submit({'type': 'event', 'sequence': f'e:{serial}', 'event': 'event.action',
                              'data': {'subscription': 'o:1', 'action': {'request': f'h:{serial}'}}})
        self.assertEqual([received.get(timeout=2)[:2] for _ in range(2)],
                         [('event.action', 'e:0'), ('event.action', 'e:1')])

    def test_schema_covers_query_viewport_and_reliable_action_fixtures(self):
        from jsonschema import Draft202012Validator
        schema = json.loads(Path(__file__).with_name('runyte-experimental-2.schema.json').read_text())
        fixtures = json.loads(Path(__file__).with_name('epoch2-fixtures.json').read_text())
        for fixture in fixtures:
            message = fixture['message']
            if message.get('id') in ('p:950', 'p:951', 'p:952', 'p:953', 'p:954', 'h:950') or message.get('sequence') in ('e:950', 'e:951', 'e:952'):
                validator = Draft202012Validator({'$defs': schema['$defs'],
                                                  '$ref': f'#/$defs/{fixture["direction"]}Message'})
                self.assertEqual(list(validator.iter_errors(message)), [], message)


class HeldWorker:
    def __init__(self):
        self.calls = []
    def submit(self, function):
        self.calls.append(function)


class CatalogTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('query_catalog_test', Path(__file__).with_name('catalog.py'))
        self.catalog = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.catalog)
        self.real_worker = self.catalog.worker
        self.catalog.worker = HeldWorker()
        self.catalog.view, self.catalog.revision = 'v:1', 'm:1'
        self.catalog.current = self.catalog.model('')
        self.context = {'view': 'v:1', 'model_revision': 'm:1', 'invocation': 'h:1', 'rows': ['record-0001']}
        self.calls = []
        def request(method, **params):
            self.calls.append((method, params))
            return {'surface': 'u:1'}
        self.catalog.app.request = request

    def tearDown(self):
        self.real_worker.shutdown(wait=True, cancel_futures=True)
        shutdown(self.catalog.app)

    def test_filter_prompt_and_cancel_do_not_start_queries(self):
        self.catalog.filter_prompt(self.context)
        self.assertEqual(self.calls[0][0], 'ui.prompt')
        self.catalog.submitted({'surface': 'u:1', 'accepted': False, 'values': {}})
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.catalog.worker.calls, [])

    def test_filter_submit_is_fresh_intent_after_model_changed_while_prompt_open(self):
        self.catalog.filter_prompt(self.context)
        self.catalog.revision = 'm:2'
        self.catalog.query = {'revision': 'qv:2', 'text': 'old', 'pending': False}
        def set_query(view, revision, text, **preconditions):
            self.calls.append(('query', (view, revision, text, preconditions)))
            return {'query': {'revision': 'qv:3', 'text': text, 'pending': True}}
        self.catalog.app.set_query = set_query
        self.catalog.submitted({'surface': 'u:1', 'accepted': True, 'values': {'query': ''}})
        self.assertEqual(self.calls[-1], ('query', ('v:1', 'm:2', '', {'expected_query_revision': 'qv:2'})))

    def observe_query(self, revision, text):
        candidate = {'revision': revision, 'text': text, 'pending': True}
        self.catalog.query = candidate
        self.catalog.observed('event.changed', 'e:1', {'sources': [
            {'source': {'kind': 'view', 'view': 'v:1'},
             'state': {'kind': 'view', 'revision': 'm:1', 'query': candidate}}]})

    def test_rapid_queries_keep_one_worker_and_only_latest_pending_intent(self):
        self.observe_query('qv:1', 'Topic 01')
        self.observe_query('qv:2', 'Topic 02')
        self.observe_query('qv:3', 'Topic 03')
        self.assertEqual(len(self.catalog.worker.calls), 1)
        self.assertEqual(self.catalog.pending_search[2]['revision'], 'qv:3')
        published = []
        def publish(view, revision, model, **preconditions):
            published.append((model, preconditions))
            return {'revision': 'm:2', 'query': {'revision': 'qv:3', 'text': 'Topic 03', 'pending': False}}
        self.catalog.app.publish_model = publish
        with patch.object(self.catalog.time, 'sleep', lambda _: None):
            self.catalog.worker.calls[0]()
        self.assertEqual(len(published), 1)
        self.assertEqual(published[0][1], {'expected_query_revision': 'qv:3'})
        self.assertTrue(all('Topic 03' in row['text'] for row in published[0][0]['rows']))
        self.assertFalse(self.catalog.search_running)

    def test_stale_result_preserves_current_model_and_newer_pending_query(self):
        self.observe_query('qv:1', 'Topic 01')
        newer = {'revision': 'qv:2', 'text': 'Topic 02', 'pending': True}
        self.catalog.query = newer
        old = self.catalog.current
        def refuse(*args, **kwargs):
            raise PluginError('stale', 'Query changed')
        self.catalog.app.publish_model = refuse
        with patch.object(self.catalog.time, 'sleep', lambda _: None):
            self.catalog.worker.calls[0]()
        self.assertIs(self.catalog.current, old)
        self.assertEqual(self.catalog.revision, 'm:1')
        self.assertEqual(self.catalog.query, newer)


if __name__ == '__main__':
    unittest.main()
