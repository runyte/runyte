# SPDX-License-Identifier: MPL-2.0
"""Disk snapshot, physical approval and bounded upload lifecycle regressions."""
import hashlib
import os
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from application import PluginError
from check_remote_operations import Port as OperationPort
from remote_upload import MAX_BYTES, Uploads, snapshot_source
from transport import TransportError


class Port(OperationPort):
    def request(self, method, **params):
        if method == 'ui.form':
            self.calls.append((method, params))
            self.serial += 1
            result = {'surface': f'id:{self.serial}'}
            if self.hook:
                self.hook(method, result)
            return result
        return super().request(method, **params)


class Transport:
    label = 'FTPS · test'
    connection_id = 'endpoint'

    def __init__(self):
        self.calls = []
        self.prepare_hook = self.upload_hook = None

    def prepare_upload(self, destination, data, cancel=None):
        self.calls.append(('prepare', destination, data))
        if self.prepare_hook:
            self.prepare_hook(cancel)
        return SimpleNamespace(connection_id=self.connection_id, destination=destination,
            bytes=len(data), sha256=hashlib.sha256(data).hexdigest(), expected_version=None,
            warnings=('Remote checks are best effort; no undo.',))

    def upload(self, prepared, data, cancel=None, progress=None):
        self.calls.append(('upload', prepared, data))
        if self.upload_hook:
            return self.upload_hook(cancel, progress)
        if cancel.is_set():
            raise TransportError('cancelled', 'Cancelled')
        if progress:
            for value in range(101):
                progress(value)
        return 'applied'


class UploadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = b'\x00\xff' + '猫\r\n'.encode()
        (self.root / 'source').write_bytes(self.data)
        self.port, self.transport, self.phases = Port(), Transport(), []
        self.uploads = Uploads(self.port, self.transport, self.phases.append, self.root)
        self.context = {'invocation': 'fresh:1'}

    def submit(self, accepted=True, **values):
        return self.uploads.submitted({'surface': self.uploads.flow['surface'], 'accepted': accepted,
                                      'invocation': 'fresh:2', 'values': values})

    def prepare(self, source='source', destination='new'):
        self.uploads.start(self.context, '/r')
        flow = self.uploads.flow
        self.assertEqual(self.submit(source=source, destination=destination), {'job': flow['job']})
        self.assertTrue(flow['done'].wait(3))
        self.assertEqual(flow['phase'], 'ready')
        return flow

    def apply(self):
        flow = self.uploads.flow
        self.uploads.confirm(self.context)
        result = self.submit()
        self.assertEqual(result, {'job': flow['job']})
        self.assertTrue(flow['done'].wait(3))
        return flow

    def test_binary_snapshot_is_frozen_and_only_exact_fresh_confirmation_uploads_once(self):
        flow = self.prepare()
        (self.root / 'source').write_bytes(b'changed after ready')
        self.assertEqual(self.transport.calls, [('prepare', '/r/new', self.data)])
        self.assertIsNone(self.uploads.submitted({'surface': 'foreign', 'accepted': True}))
        self.uploads.confirm({'invocation': 'fresh:99'})
        method, params = self.port.calls[-1]
        self.assertEqual((method, params['invocation']), ('ui.confirm', 'fresh:99'))
        self.assertIn('"source" → "/r/new"', params['message'])
        self.assertIn(f'{len(self.data)} disk bytes', params['message'])
        surface = flow['surface']
        self.submit()
        self.assertTrue(flow['done'].wait(3))
        self.assertEqual(self.transport.calls[-1][2], self.data)
        self.assertIsNone(self.uploads.submitted({'surface': surface, 'accepted': True}))
        self.assertEqual(sum(call[0] == 'upload' for call in self.transport.calls), 1)
        self.assertEqual(self.phases[-1], 'completed')
        self.assertFalse(any(isinstance(value, bytes) for _, params in self.port.calls for value in params.values()))

    def test_empty_and_unicode_named_disk_files_are_exact(self):
        (self.root / '文').write_bytes(b'')
        self.prepare('文')
        self.apply()
        self.assertEqual(self.transport.calls[-1][2], b'')
        self.assertEqual(self.phases[-1], 'completed')

    def test_cancel_prompt_or_confirmation_performs_no_upload(self):
        self.uploads.start(self.context, '/r')
        self.submit(False)
        self.assertEqual(self.transport.calls, [])
        self.prepare()
        self.uploads.confirm(self.context)
        self.submit(False)
        self.assertEqual(len(self.transport.calls), 1)
        self.assertIsNone(self.uploads.flow)

    def test_stale_confirmation_can_only_represent_same_snapshot_with_new_invocation(self):
        flow = self.prepare()
        self.port.failure = ('ui.confirm', 'context_changed')
        with self.assertRaises(PluginError):
            self.uploads.confirm(self.context)
        self.assertIs(self.uploads.flow, flow)
        self.assertEqual(flow['phase'], 'ready')
        self.port.failure = None
        self.apply()
        self.assertEqual(sum(call[0] == 'prepare' for call in self.transport.calls), 1)

    def test_progress_is_coarse_and_100_follows_only_acknowledged_promotion(self):
        self.prepare()
        def upload(cancel, progress):
            for value in range(101):
                progress(value)
            updates = [p['progress'] for m, p in self.port.calls if m == 'job.update']
            self.assertEqual(updates, list(range(0, 91, 10)))
            return 'applied'
        self.transport.upload_hook = upload
        self.apply()
        updates = [p['progress'] for m, p in self.port.calls if m == 'job.update']
        self.assertEqual(updates, list(range(0, 101, 10)))

    def test_known_conflict_can_retry_but_unknown_retains_exact_snapshot_and_blocks_cancel(self):
        for error, expected in [(TransportError('conflict', 'Changed'), 'failed'),
                                (TransportError('timeout', 'Lost', settled=False), 'outcome_unknown')]:
            with self.subTest(state=expected):
                self.setUp()
                flow = self.prepare()
                def upload(cancel, progress):
                    raise error
                self.transport.upload_hook = upload
                self.apply()
                self.assertEqual(self.phases[-1], expected)
                if expected == 'failed':
                    self.assertIsNone(self.uploads.flow)
                else:
                    self.assertIs(self.uploads.flow, flow)
                    self.assertEqual(flow['data'], self.data)
                    with self.assertRaises(PluginError):
                        self.uploads.cancel({})
                    with self.assertRaises(PluginError):
                        self.uploads.start(self.context, '/r')
                    self.uploads.event('job.changed', {'job': flow['job'], 'state': 'cancelled'})
                    self.assertIs(self.uploads.flow, flow)

    def test_host_cancel_before_success_ack_gets_terminal_unknown_and_retains_flow(self):
        flow = self.prepare()
        request = self.port.request
        def cancelling(method, **params):
            result = request(method, **params)
            if method == 'job.finish' and params['state'] == 'succeeded':
                raise PluginError('cancelled', 'Already cancelling')
            return result
        self.port.request = cancelling
        self.apply()
        self.assertEqual([p['state'] for m, p in self.port.calls if m == 'job.finish'],
                         ['succeeded', 'outcome_unknown'])
        self.assertEqual(flow['phase'], 'outcome_unknown')
        self.assertEqual(flow['data'], self.data)

    def test_early_job_cancel_and_retained_plan_deadline_prevent_upload(self):
        def hook(method, result):
            if method == 'job.create':
                self.uploads.event('job.cancel_requested', result)
        self.port.hook = hook
        self.uploads.start(self.context, '/r')
        with self.assertRaises(PluginError):
            self.submit(source='source', destination='new')
        self.assertEqual(self.transport.calls, [])
        self.port.hook = None
        flow = self.prepare()
        self.uploads.event('job.changed', {'job': flow['job'], 'state': 'cancelled'})
        self.assertTrue(flow['done'].wait(3))
        self.assertIsNone(self.uploads.flow)
        self.assertEqual(len(self.transport.calls), 1)

    def test_late_prompt_response_is_dismissed_after_cancel(self):
        entered, resume = threading.Event(), threading.Event()
        def hook(method, result):
            if method == 'ui.form':
                entered.set()
                resume.wait(3)
        self.port.hook = hook
        worker = threading.Thread(target=self.uploads.start, args=(self.context, '/r'))
        worker.start()
        self.assertTrue(entered.wait(2))
        self.uploads.cancel({})
        resume.set()
        worker.join(3)
        self.assertFalse(worker.is_alive())
        self.assertEqual(self.port.calls[-1][0], 'ui.dismiss')
        self.assertIsNone(self.uploads.flow)

    def test_unrenderable_exact_confirmation_is_refused_before_ready(self):
        self.uploads.start(self.context, '/r')
        flow = self.uploads.flow
        self.submit(source='source', destination='猫' * 100)
        self.assertTrue(flow['done'].wait(3))
        self.assertEqual(self.phases[-1], 'failed')
        self.assertFalse(any(m == 'ui.confirm' for m, _ in self.port.calls))

    def test_source_growth_and_replacement_during_read_are_refused(self):
        read = os.read
        for mutation in ('grow', 'replace'):
            with self.subTest(mutation=mutation):
                (self.root / 'source').write_bytes(self.data)
                changed = False
                def changing(fd, count):
                    nonlocal changed
                    data = read(fd, count)
                    if not changed:
                        changed = True
                        if mutation == 'grow':
                            with (self.root / 'source').open('ab') as target:
                                target.write(b'x')
                        else:
                            (self.root / 'other').write_bytes(self.data)
                            os.replace(self.root / 'other', self.root / 'source')
                    return data
                with patch('remote_upload.os.read', changing), self.assertRaises(PluginError) as raised:
                    snapshot_source(self.root, 'source', threading.Event())
                self.assertEqual(raised.exception.code, 'stale')

    def test_source_containment_symlinks_fifo_and_size_limits(self):
        (self.root / 'link').symlink_to(self.root / 'source')
        (self.root / 'dirlink').symlink_to(self.root, target_is_directory=True)
        os.mkfifo(self.root / 'pipe')
        with (self.root / 'big').open('wb') as target:
            target.truncate(MAX_BYTES + 1)
        for source in ('../source', '/source', 'link', 'dirlink/source', 'pipe', 'big', 'source/../source', '.', '\ud800'):
            with self.subTest(source=repr(source)), self.assertRaises(PluginError):
                snapshot_source(self.root, source, threading.Event())
        self.assertEqual(snapshot_source(self.root, 'source', threading.Event()), self.data)


if __name__ == '__main__':
    unittest.main()
