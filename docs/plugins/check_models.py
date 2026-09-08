# SPDX-License-Identifier: MPL-2.0
"""SDK model transfer, immutable reads and dashboard checks.

Requires the development-only jsonschema package for structural conformance.
"""
import json
import unittest
from application import Application, PluginError, MODEL_CHUNK, MODEL_LIMIT


class ModelPort:
    def __init__(self, model):
        self.model, self.calls = model, []
        self.staged = bytearray()
        self.expected = 0
        self.fault = None

    def request(self, method, **params):
        self.calls.append((method, params))
        if self.fault and method == self.fault[0]:
            raise PluginError(self.fault[1], 'Injected refusal')
        if method == 'view.stage.open':
            self.expected = params['bytes']
            return {'stage': 's:1', 'bytes': self.expected}
        if method == 'view.stage.write':
            assert params['offset'] == len(self.staged)
            data = params['text'].encode('utf-8')
            assert len(data) <= MODEL_CHUNK
            self.staged.extend(data)
            return {'offset': len(self.staged)}
        if method == 'view.stage.commit':
            assert len(self.staged) == self.expected
            self.model = json.loads(self.staged)
            return {'view': 'v:1', 'revision': 'm:2', 'bytes': self.expected, 'rows': 1}
        if method == 'view.get':
            self.snapshot = json.dumps(self.model, ensure_ascii=False, separators=(',', ':')).encode()
            return {'view': 'v:1', 'revision': 'm:2', 'bytes': len(self.snapshot), 'rows': 1}
        if method == 'view.snapshot.open':
            assert params['expected_revision'] == 'm:2'
            return {'snapshot': 'r:1', 'revision': 'm:2', 'bytes': len(self.snapshot)}
        if method == 'view.snapshot.read':
            offset = params['offset']
            end = min(offset + params['limit'], len(self.snapshot))
            while end < len(self.snapshot) and self.snapshot[end] & 0xc0 == 0x80:
                end -= 1
            return {'offset': offset, 'text': self.snapshot[offset:end].decode(), 'eof': end == len(self.snapshot)}
        return {}


class ModelTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Models', [], ['views'])
        self.model = {'title': 'Unicode', 'purpose': 'document', 'rows': [
            {'id': 'row', 'text': 'é猫' * 300000, 'role': 'ordinary'}]}
        self.port = ModelPort(self.model)
        self.app.request = self.port.request

    def tearDown(self):
        for executor in (self.app._executor, self.app._resource_executor,
                         self.app._control, self.app._observations, self.app._validation_executor):
            executor.shutdown(wait=True, cancel_futures=True)

    def test_small_model_and_patch_preserve_single_message_operations(self):
        model = {'title': 'Small', 'purpose': 'list', 'rows': []}
        self.app.publish_model('v:1', 'm:1', model)
        self.app.patch_view('v:1', 'm:1', [{'kind': 'remove', 'ids': ['row']}])
        self.assertEqual([method for method, _ in self.port.calls], ['view.publish', 'view.patch'])
        self.assertEqual(self.port.calls[0][1]['model'], model)
        self.assertEqual(self.port.calls[1][1]['expected_revision'], 'm:1')

    def test_large_unicode_model_stages_exact_bytes_and_closes_after_commit(self):
        result = self.app.publish_model('v:1', 'm:1', self.model)
        self.assertEqual(result['revision'], 'm:2')
        self.assertEqual(self.port.model, self.model)
        self.assertGreater(len(self.port.calls), 3)
        self.assertEqual(self.port.calls[0][0], 'view.stage.open')
        self.assertEqual(self.port.calls[-2][0], 'view.stage.commit')
        self.assertEqual(self.port.calls[-1][0], 'view.stage.close')

    def test_large_patch_stages_patch_document_without_target_duplication(self):
        operations = [{'kind': 'update', 'row': self.model['rows'][0]}]
        self.app.patch_view('v:1', 'm:1', operations)
        self.assertEqual(self.port.calls[0][1]['kind'], 'patch')
        self.assertEqual(self.port.model, {'operations': operations})

    def test_failed_commit_closes_stage_without_retrying_mutation(self):
        self.port.fault = ('view.stage.commit', 'stale')
        with self.assertRaises(PluginError) as error:
            self.app.publish_model('v:1', 'm:1', self.model)
        self.assertEqual(error.exception.code, 'stale')
        self.assertEqual([method for method, _ in self.port.calls].count('view.stage.commit'), 1)
        self.assertEqual(self.port.calls[-1][0], 'view.stage.close')

    def test_oversized_update_fails_before_host_admission(self):
        with self.assertRaises(PluginError) as error:
            self.app.publish_model('v:1', 'm:1', {'text': 'x' * MODEL_LIMIT})
        self.assertEqual(error.exception.code, 'limit_exceeded')
        self.assertFalse(self.port.calls)

    def test_large_read_uses_one_immutable_utf8_snapshot_and_releases_it(self):
        result = self.app.get_model('v:1')
        self.assertEqual(result, {'view': 'v:1', 'revision': 'm:2', 'model': self.model})
        self.assertEqual(self.port.calls[-1][0], 'view.snapshot.close')

    def test_read_refusal_releases_snapshot_without_restarting(self):
        self.port.fault = ('view.snapshot.read', 'closed')
        with self.assertRaises(PluginError) as error:
            self.app.get_model('v:1')
        self.assertEqual(error.exception.code, 'closed')
        self.assertEqual(self.port.calls[-1][0], 'view.snapshot.close')
        self.assertEqual([method for method, _ in self.port.calls].count('view.snapshot.open'), 1)


    def test_failed_stage_write_and_wrong_ack_release_without_commit(self):
        original = self.port.request
        for fault in ('refused', 'offset'):
            with self.subTest(fault=fault):
                self.port.calls.clear()
                self.port.staged.clear()
                def request(method, **params):
                    if method == 'view.stage.write':
                        self.port.calls.append((method, params))
                        if fault == 'refused':
                            raise PluginError('busy', 'Refused chunk')
                        return {'offset': params['offset']}
                    return original(method, **params)
                self.app.request = request
                with self.assertRaises(PluginError):
                    self.app.publish_model('v:1', 'm:1', self.model)
                methods = [method for method, _ in self.port.calls]
                self.assertNotIn('view.stage.commit', methods)
                self.assertEqual(methods[-1], 'view.stage.close')

    def test_malformed_snapshot_chunks_are_refused_and_released(self):
        original = self.port.request
        for fault in ('wrong_offset', 'empty_nonfinal', 'short_final', 'too_large'):
            with self.subTest(fault=fault):
                self.port.calls.clear()
                def request(method, **params):
                    if method == 'view.snapshot.read':
                        self.port.calls.append((method, params))
                        return {'offset': 1 if fault == 'wrong_offset' else 0,
                                'text': 'x' * (MODEL_CHUNK + 1) if fault == 'too_large' else '',
                                'eof': fault == 'short_final'}
                    return original(method, **params)
                self.app.request = request
                with self.assertRaises(PluginError) as error:
                    self.app.get_model('v:1')
                self.assertEqual(error.exception.code, 'invalid_argument')
                self.assertEqual(self.port.calls[-1][0], 'view.snapshot.close')
                self.assertEqual(sum(method == 'view.snapshot.read' for method, _ in self.port.calls), 1)

    def test_snapshot_revision_race_does_not_restart_or_read_a_new_revision(self):
        self.port.fault = ('view.snapshot.open', 'stale')
        with self.assertRaises(PluginError) as error:
            self.app.get_model('v:1')
        self.assertEqual(error.exception.code, 'stale')
        self.assertEqual([method for method, _ in self.port.calls], ['view.get', 'view.snapshot.open'])


class DashboardTests(unittest.TestCase):
    def test_patch_refusal_preserves_local_model_then_success_adopts_exact_rows(self):
        import dashboard
        original = dashboard.app, dashboard.view, dashboard.revision, dashboard.current
        candidate = dashboard.model(3)
        class Port:
            fail = True
            def patch_view(self, view, revision, operations, header):
                self.operations = operations
                if self.fail:
                    raise PluginError('busy', 'Publication paced')
                return {'revision': 'm:2'}
        port = Port()
        try:
            dashboard.app, dashboard.view, dashboard.revision, dashboard.current = port, 'v:1', 'm:1', candidate
            context = {'view': 'v:1', 'model_revision': 'm:1', 'rows': ['item-0001']}
            with self.assertRaises(PluginError):
                dashboard.update(context, 'toggle')
            self.assertIs(dashboard.current, candidate)
            self.assertEqual(candidate['rows'][1]['cells'][1]['text'], 'Ready')
            self.assertEqual(dashboard.revision, 'm:1')
            port.fail = False
            dashboard.update(context, 'toggle')
            self.assertEqual(dashboard.current['rows'][1]['cells'][1]['text'], 'Done')
            self.assertEqual(dashboard.current['rows'][0], candidate['rows'][0])
            self.assertEqual(port.operations[0]['row']['id'], 'item-0001')
            self.assertEqual(dashboard.revision, 'm:2')
        finally:
            dashboard.app, dashboard.view, dashboard.revision, dashboard.current = original

    def test_dashboard_models_and_stage_empty_refusals_match_schema(self):
        import dashboard
        from pathlib import Path
        from jsonschema import Draft202012Validator
        schema = json.loads(Path(__file__).with_name('runyte-experimental-2.schema.json').read_text())
        def validator(name):
            return Draft202012Validator({'$defs': schema['$defs'], '$ref': f'#/$defs/{name}'})
        for count in (12, 8000):
            model = dashboard.model(count)
            validator('model').validate(model)
            encoded = json.dumps(model, ensure_ascii=False, separators=(',', ':')).encode()
            self.assertLess(len(encoded), MODEL_LIMIT)
            self.assertEqual(len(encoded) > 1024 * 1024, count == 8000)
            self.assertEqual(len({row['id'] for row in model['rows']}), count)
        for method, params in [
            ('view.stage.open', {'view': 'v:1', 'expected_revision': 'm:1', 'kind': 'model', 'bytes': 0}),
            ('view.stage.write', {'stage': 'vs:1', 'offset': 0, 'text': ''}),
        ]:
            message = {'type': 'request', 'id': 'p:1', 'method': method, 'params': params}
            self.assertFalse(validator(method).is_valid(message))


if __name__ == '__main__':
    unittest.main(verbosity=2)
