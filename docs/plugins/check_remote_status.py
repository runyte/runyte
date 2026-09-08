# SPDX-License-Identifier: MPL-2.0
"""Visible remote download phases, bounded retries and stale-view publication."""
import json
import unittest

from application import PluginError
from check_sftp_ui import Port, Provider, Transport
from sftp import SftpApplication, MAX_MODEL_BYTES
from remote_status import STATUS_ROW


class StatusTests(unittest.TestCase):
    def setUp(self):
        self.port = Port()
        self.ui = SftpApplication(Transport(), Provider(), 'custom-sftp', self.port)
        self.ui.browse({'arguments': {'path': '.'}, 'invocation': 'h:1'})
        self.port.calls.clear()

    def settle(self):
        self.assertTrue(self.ui.download_status.idle.wait(2), 'Status publisher did not settle')

    def publications(self):
        return [params for method, params in self.port.calls if method == 'view.publish']

    def test_ready_is_visible_with_exact_command_and_stable_file_rows(self):
        original, revision = dict(self.ui.entries), self.ui.revision
        self.ui.download_status('ready')
        self.settle()
        result = self.publications()[-1]
        self.assertEqual(result['expected_revision'], revision)
        self.assertEqual(result['model']['rows'][-1], {
            'id': STATUS_ROW, 'text': 'Download ready · run :plugin.custom-sftp.confirm-download',
            'role': 'heading'})
        self.assertEqual({row['id'] for row in result['model']['rows'][:-1]}, set(original))
        self.assertEqual(self.ui.entries, original)
        self.assertNotEqual(self.ui.revision, revision)
        self.assertLessEqual(len(json.dumps(result['model'], ensure_ascii=False).encode()), MAX_MODEL_BYTES)
        self.assertEqual([method for method, _ in self.port.calls], ['view.publish'])

    def test_busy_ready_retries_twice_then_becomes_visible_and_idle(self):
        request = self.port.request
        attempts = []
        def busy_twice(method, **params):
            if method == 'view.publish':
                attempts.append(params)
                if len(attempts) <= 2:
                    raise PluginError('busy', 'Publication rate limit')
            return request(method, **params)
        self.port.request = busy_twice
        self.ui.download_status('ready')
        self.settle()
        self.assertEqual(len(attempts), 3)
        self.assertTrue(self.publications()[-1]['model']['rows'][-1]['text'].startswith('Download ready'))
        self.assertFalse(self.ui.download_status.active)
        self.assertIsNone(self.ui.download_status.pending)

    def test_latest_flow_phase_coalesces_without_overwriting_new_view(self):
        with self.ui.lock:
            self.ui.download_status('ready')
            self.ui.download_status('transferring')
            self.ui.download_status('ready')
        self.settle()
        self.assertEqual(len(self.publications()), 1)
        self.assertTrue(self.publications()[0]['model']['rows'][-1]['text'].startswith('Download ready'))
        self.port.calls.clear()
        with self.ui.lock:
            self.ui.download_status('failed')
            self.ui.view, self.ui.revision = 'new-view', 'new-revision'
        self.settle()
        self.assertEqual(self.publications(), [])
        self.assertEqual((self.ui.view, self.ui.revision), ('new-view', 'new-revision'))

    def test_failed_cancelled_and_completed_phases_update_or_remove_status_only(self):
        files = dict(self.ui.entries)
        for phase, text, role in [('transferring', 'Downloading remote file', 'muted'),
                                  ('failed', 'Download failed', 'error'),
                                  ('cancelled', 'Download cancelled', 'muted')]:
            self.ui.download_status(phase)
            self.settle()
            row = self.publications()[-1]['model']['rows'][-1]
            self.assertTrue(row['text'].startswith(text))
            self.assertEqual(row['role'], role)
            self.assertEqual(self.ui.entries, files)
        self.ui.download_status('completed')
        self.settle()
        self.assertNotIn(STATUS_ROW, [row['id'] for row in self.publications()[-1]['model']['rows']])

    def test_closed_view_is_not_recreated_and_explicit_refresh_keeps_ready_status(self):
        self.ui.download_status('ready')
        self.settle()
        self.ui.refresh({'view': self.ui.view, 'model_revision': self.ui.revision})
        self.assertEqual(self.publications()[-1]['model']['rows'][-1]['id'], STATUS_ROW)
        self.ui.event('view.closed', {'view': self.ui.view})
        self.port.calls.clear()
        self.ui.download_status('cancelled')
        self.settle()
        self.assertEqual(self.port.calls, [])
        self.assertIsNone(self.ui.view)

    def test_permanent_busy_exhausts_attempts_without_leaving_a_worker(self):
        attempts = []
        def busy(method, **params):
            attempts.append(method)
            raise PluginError('busy', 'Still busy')
        self.port.request = busy
        self.ui.download_status('ready')
        self.settle()
        self.assertEqual(attempts, ['view.publish'] * 3)
        self.assertFalse(self.ui.download_status.active)
        self.assertIsNone(self.ui.download_status.pending)

    def test_operation_and_download_rows_preserve_each_other_and_exact_actions(self):
        original = set(self.ui.entries)
        self.ui.download_status('ready')
        self.settle()
        self.ui.operation_status('ready')
        self.assertTrue(self.ui.operation_status.idle.wait(2))
        rows = self.publications()[-1]['model']['rows']
        self.assertEqual({row['id'] for row in rows}, original | {STATUS_ROW, 'operation-status'})
        self.assertEqual(rows[-1]['text'], 'Remote operation ready · run :plugin.custom-sftp.confirm-operation')
        self.ui.operation_status('outcome_unknown')
        self.assertTrue(self.ui.operation_status.idle.wait(2))
        self.assertEqual(self.publications()[-1]['model']['rows'][-1]['role'], 'error')
        self.assertIn('do not retry', self.publications()[-1]['model']['rows'][-1]['text'])
        self.ui.download_status('completed')
        self.settle()
        rows = self.publications()[-1]['model']['rows']
        self.assertEqual({row['id'] for row in rows}, original | {'operation-status'})
        self.assertEqual(self.ui.entries.keys(), dict.fromkeys(original).keys())

    def test_upload_download_and_operation_phases_keep_all_bounded_rows(self):
        original = set(self.ui.entries)
        for status in (self.ui.download_status, self.ui.operation_status, self.ui.upload_status):
            status('ready')
            self.assertTrue(status.idle.wait(2))
        rows = self.publications()[-1]['model']['rows']
        self.assertEqual({row['id'] for row in rows}, original | {STATUS_ROW, 'operation-status', 'upload-status'})
        self.assertEqual(rows[-1]['text'], 'Upload ready · run :plugin.custom-sftp.confirm-upload')
        self.assertLessEqual(len(json.dumps(self.publications()[-1]['model']).encode()), MAX_MODEL_BYTES)
        for phase in ('preparing', 'confirming', 'applying', 'completed', 'failed', 'cancelled', 'outcome_unknown'):
            self.ui.upload_status(phase)
            self.assertTrue(self.ui.upload_status.idle.wait(2))
            self.assertEqual({row['id'] for row in self.publications()[-1]['model']['rows']},
                             original | {STATUS_ROW, 'operation-status', 'upload-status'})
        self.assertIn('restart plugin', self.publications()[-1]['model']['rows'][-1]['text'])
        self.assertEqual(self.publications()[-1]['model']['rows'][-1]['role'], 'error')
        self.assertTrue(all(method == 'view.publish' for method, _ in self.port.calls))

    def test_operation_completion_keeps_a_refresh_action_without_background_network_io(self):
        self.ui.operation_status('completed')
        self.assertTrue(self.ui.operation_status.idle.wait(2))
        self.assertEqual(self.publications()[-1]['model']['rows'][-1]['text'],
                         'Remote operation completed · run :plugin.custom-sftp.refresh')
        self.assertEqual([method for method, _ in self.port.calls], ['view.publish'])


if __name__ == '__main__':
    unittest.main()
