# SPDX-License-Identifier: MPL-2.0
"""Binary download orchestration checks through a fake public host port."""
import hashlib
from pathlib import Path
import tempfile
import threading
import unittest

from application import PluginError
from remote_download import Downloads


class Port:
    def __init__(self, root):
        self.root, self.calls = root, []
        self.failure = None
        self.finished = threading.Event()
        self.serial = 0

    def request(self, method, **params):
        self.calls.append((method, params))
        if self.failure and self.failure[0] == method:
            raise PluginError(self.failure[1], 'Injected host refusal')
        if method == 'ui.prompt':
            return {'surface': 's:1'}
        if method == 'job.create':
            self.serial += 1
            return {'job': f'j:{self.serial}'}
        if method == 'staging.create':
            path = self.root / 'stage'
            path.write_bytes(b'')
            return {'staging': 't:1', 'path': str(path)}
        if method == 'filesystem.list':
            return {'directory': 'd:1', 'revision': 'r:1'}
        if method == 'staging.prepare':
            assert params['sha256'] == hashlib.sha256((self.root / 'stage').read_bytes()).hexdigest()
            return {'plan': 'p:1', 'operations': ['Create download']}
        if method == 'job.finish':
            self.finished.set()
        return {}


class Transport:
    def __init__(self):
        self.data, self.hook = b'\x00\xff\x80\r\n', None

    def download(self, source, stage, expected_bytes, cancel, progress):
        if self.hook:
            self.hook()
        if cancel.is_set():
            raise PluginError('cancelled', 'Cancelled during transfer')
        assert expected_bytes == len(self.data)
        Path(stage).write_bytes(self.data)
        progress(len(self.data))
        return hashlib.sha256(self.data).hexdigest()


class DownloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='runyte-download-check-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.port, self.transport = Port(self.root), Transport()
        self.downloads = Downloads(self.port, self.transport)
        self.entry = {'path': '/remote/binary', 'kind': 'file', 'size': len(self.transport.data)}
        self.callback = {'surface': 's:1', 'accepted': True, 'invocation': 'fresh:2',
                         'values': {'destination': 'nested/result.bin'}}

    def prompt(self):
        self.downloads.prompt({'invocation': 'original:1'}, self.entry)

    def transfer(self):
        flow = self.downloads.flow
        result = self.downloads.submitted(self.callback)
        self.assertEqual(result, {'job': 'j:1'})
        self.assertTrue(flow['done'].wait(2), 'Download worker did not finish')
        return flow

    def test_native_prompt_cancellation_never_creates_a_job_or_stage(self):
        self.prompt()
        self.downloads.submitted({**self.callback, 'accepted': False})
        self.assertEqual([method for method, _ in self.port.calls], ['ui.prompt'])
        self.assertIsNone(self.downloads.flow)

    def test_delayed_prompt_reply_cannot_block_cancellation_and_late_surface_is_dismissed(self):
        entered, resume, handled = threading.Event(), threading.Event(), threading.Event()
        request = self.port.request
        def delayed(method, **params):
            if method == 'ui.prompt':
                entered.set()
                self.assertTrue(resume.wait(2))
            return request(method, **params)
        self.port.request = delayed
        worker = threading.Thread(target=self.prompt)
        worker.start()
        self.assertTrue(entered.wait(1))
        def cancel_event():
            self.downloads.event('job.cancel_requested', {'job': 'unrelated'})
            handled.set()
        controller = threading.Thread(target=cancel_event)
        controller.start()
        try:
            self.assertTrue(handled.wait(0.5), 'Cancellation handler waited on prompt reply')
            self.downloads.cancel({})
        finally:
            resume.set()
            worker.join(2)
            controller.join(2)
        self.assertFalse(worker.is_alive())
        self.assertIn(('ui.dismiss', {'surface': 's:1'}), self.port.calls)
        self.assertIsNone(self.downloads.flow)

    def test_binary_staging_uses_exact_bytes_and_requires_explicit_fresh_confirmation(self):
        self.prompt()
        self.transfer()
        self.assertEqual((self.root / 'stage').read_bytes(), self.transport.data)
        self.assertFalse((self.root / 'nested' / 'result.bin').exists())
        self.assertIn(('staging.create', {'job': 'j:1', 'bytes': len(self.transport.data)}), self.port.calls)
        self.assertIn(('filesystem.list', {'path': 'nested', 'offset': 0, 'limit': 1,
                                           'expected_revision': None}), self.port.calls)
        prepared = next(params for method, params in self.port.calls if method == 'staging.prepare')
        self.assertEqual(prepared['destination'], 'nested/result.bin')
        self.assertFalse(any(method in ('filesystem.apply', 'job.finish') for method, _ in self.port.calls))
        self.downloads.confirm({'invocation': 'fresh:9'})
        self.assertIn(('filesystem.apply', {'plan': 'p:1', 'invocation': 'fresh:9'}), self.port.calls)
        self.assertEqual(self.port.calls[-1], ('job.finish', {'job': 'j:1', 'state': 'succeeded'}))
        self.assertIsNone(self.downloads.flow)

    def test_stale_foreground_retains_plan_until_an_explicit_fresh_confirmation(self):
        self.prompt()
        self.transfer()
        self.port.failure = ('filesystem.apply', 'context_changed')
        self.downloads.confirm({'invocation': 'expired:2'})
        self.assertEqual(self.downloads.flow['phase'], 'prepared')
        self.assertFalse(any(method in ('job.finish', 'filesystem.cancel') for method, _ in self.port.calls))
        self.port.failure = None
        self.downloads.confirm({'invocation': 'fresh:9'})
        self.assertIn(('filesystem.apply', {'plan': 'p:1', 'invocation': 'fresh:9'}), self.port.calls)
        self.assertEqual(sum(method == 'staging.prepare' for method, _ in self.port.calls), 1)

    def test_cancel_during_download_closes_stage_and_never_prepares_or_applies(self):
        self.prompt()
        self.transport.hook = lambda: self.downloads.event('job.cancel_requested', {'job': 'j:1'})
        self.transfer()
        self.assertIn(('staging.close', {'staging': 't:1'}), self.port.calls)
        self.assertIn(('job.finish', {'job': 'j:1', 'state': 'cancelled'}), self.port.calls)
        self.assertFalse(any(method in ('staging.prepare', 'filesystem.apply') for method, _ in self.port.calls))

    def test_cancellation_overtaking_job_creation_prevents_staging(self):
        self.prompt()
        request = self.port.request
        def overtaken(method, **params):
            result = request(method, **params)
            if method == 'job.create':
                self.downloads.event('job.cancel_requested', {'job': result['job']})
            return result
        self.port.request = overtaken
        with self.assertRaises(PluginError) as raised:
            self.downloads.submitted(self.callback)
        self.assertEqual(raised.exception.code, 'cancelled')
        self.assertFalse(any(method == 'staging.create' for method, _ in self.port.calls))
        self.assertIn(('job.finish', {'job': 'j:1', 'state': 'cancelled'}), self.port.calls)

    def test_cancel_retained_plan_reclaims_it_without_waiting_in_control_handler(self):
        self.prompt()
        self.transfer()
        self.downloads.event('job.cancel_requested', {'job': 'j:1'})
        self.assertTrue(self.port.finished.wait(2))
        self.assertIn(('filesystem.cancel', {'plan': 'p:1'}), self.port.calls)
        self.assertIn(('job.finish', {'job': 'j:1', 'state': 'cancelled'}), self.port.calls)

    def test_failure_releases_directory_snapshot_and_stage(self):
        self.prompt()
        self.port.failure = ('staging.prepare', 'conflict')
        self.transfer()
        self.assertIn(('filesystem.release', {'directory': 'd:1'}), self.port.calls)
        self.assertIn(('staging.close', {'staging': 't:1'}), self.port.calls)
        self.assertFalse(any(method == 'filesystem.apply' for method, _ in self.port.calls))
        self.assertIsNone(self.downloads.flow)

    def test_invalid_selection_size_path_and_second_flow_are_refused_before_staging(self):
        for entry in [{**self.entry, 'size': 8 * 1024 * 1024 + 1}, {**self.entry, 'size': True},
                      {**self.entry, 'kind': 'symlink'}]:
            with self.assertRaises(PluginError):
                self.downloads.prompt({'invocation': 'h:1'}, entry)
        self.prompt()
        with self.assertRaises(PluginError):
            self.prompt()
        with self.assertRaises(PluginError):
            self.downloads.submitted({**self.callback, 'values': {'destination': '../outside'}})
        self.assertFalse(any(method == 'staging.create' for method, _ in self.port.calls))

    def test_terminal_host_job_drops_pending_flow_without_recreating_it(self):
        self.prompt()
        self.transfer()
        self.downloads.event('job.changed', {'job': 'j:1', 'state': 'cancelled'})
        self.assertIsNone(self.downloads.flow)
        with self.assertRaises(PluginError):
            self.downloads.confirm({'invocation': 'h:9'})

    def test_status_reports_bounded_transfer_ready_and_confirmed_completion_phases(self):
        phases = []
        self.downloads.on_status = phases.append
        self.prompt()
        self.transfer()
        self.assertEqual(phases, ['transferring', 'ready'])
        self.downloads.confirm({'invocation': 'h:9'})
        self.assertEqual(phases, ['transferring', 'ready', 'completed'])

    def test_old_completion_cannot_clear_a_new_downloads_status(self):
        phases = []
        self.downloads.on_status = phases.append
        self.prompt()
        self.transfer()
        original = self.port.request
        entered, resume = threading.Event(), threading.Event()
        def blocked():
            entered.set()
            self.assertTrue(resume.wait(2))
        self.transport.hook = blocked
        new_flow = []
        def finishing(method, **params):
            result = original(method, **params)
            if method == 'job.finish' and params['job'] == 'j:1':
                self.downloads.event('job.changed', {'job': 'j:1', 'state': 'succeeded'})
                self.prompt()
                new_flow.append(self.downloads.flow)
                self.downloads.submitted(self.callback)
            return result
        self.port.request = finishing
        try:
            self.downloads.confirm({'invocation': 'h:9'})
            self.assertTrue(entered.wait(1))
            self.assertEqual(phases[-1], 'transferring')
        finally:
            resume.set()
            if new_flow:
                self.assertTrue(new_flow[0]['done'].wait(2))
                self.downloads.cancel({})

    def test_callback_returns_job_and_real_worker_slot_survives_logical_retirement(self):
        self.prompt()
        entered, resume = threading.Event(), threading.Event()
        def blocked():
            entered.set()
            self.assertTrue(resume.wait(2))
        self.transport.hook = blocked
        flow = self.downloads.flow
        try:
            self.assertEqual(self.downloads.submitted(self.callback), {'job': 'j:1'})
            self.assertTrue(entered.wait(1))
            self.assertTrue(self.downloads.worker_active)
            self.downloads.event('job.changed', {'job': 'j:1', 'state': 'cancelled'})
            self.assertIsNone(self.downloads.flow)
            with self.assertRaises(PluginError) as raised:
                self.prompt()
            self.assertEqual(raised.exception.code, 'busy')
        finally:
            resume.set()
            self.assertTrue(flow['done'].wait(2))
        self.assertFalse(self.downloads.worker_active)

    def test_oversized_or_invalid_unicode_destination_refuses_before_job_creation(self):
        for destination in ['猫' * 1366, '\ud800']:
            self.prompt()
            with self.assertRaises(PluginError) as raised:
                self.downloads.submitted({**self.callback, 'values': {'destination': destination}})
            self.assertEqual(raised.exception.code, 'invalid_argument')
        self.assertFalse(any(method == 'job.create' for method, _ in self.port.calls))


if __name__ == '__main__':
    unittest.main(verbosity=2)
