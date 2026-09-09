# SPDX-License-Identifier: MPL-2.0
"""Independent upload source, cancellation and approval boundary regressions."""
import hashlib
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

from application import PluginError
from check_remote_operations import Port as OperationPort
from remote_upload import Uploads, snapshot_source
from transport import PreparedUpload, TransportError


class Port(OperationPort):
    def request(self, method, **params):
        if method == 'ui.form':
            self.calls.append((method, params))
            self.serial += 1
            return {'surface': f'id:{self.serial}'}
        return super().request(method, **params)


class Transport:
    label = 'SFTP test'
    connection_id = 'review-connection'

    def __init__(self):
        self.calls = []
        self.hook = None

    def prepare_upload(self, destination, data, cancel):
        self.calls.append(('prepare', destination, data))
        return PreparedUpload(self.connection_id, destination, len(data),
                              hashlib.sha256(data).hexdigest(), None,
                              ('No undo.',))

    def upload(self, prepared, data, cancel, progress):
        self.calls.append(('upload', prepared, data))
        if self.hook:
            return self.hook()
        return 'applied'


class UploadReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='runyte-upload-review-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'workspace'
        self.root.mkdir()
        self.port, self.transport = Port(), Transport()
        self.uploads = Uploads(self.port, self.transport, workspace_root=self.root)
        self.data = b'\x00\xffdisk bytes\r\n'
        (self.root / 'file').write_bytes(self.data)

    def submit(self, surface, **extra):
        return self.uploads.submitted({'surface': surface, 'accepted': True,
                                      'invocation': 'physical:accepted', **extra})

    def start(self):
        self.uploads.start({'invocation': 'physical:form'}, '/remote')
        flow = self.uploads.flow
        self.submit(flow['surface'], values={'source': 'file', 'destination': 'target'})
        return flow

    def prepare(self):
        flow = self.start()
        self.assertTrue(flow['done'].wait(3))
        self.assertEqual(flow['phase'], 'ready')
        return flow

    def test_parent_replacement_during_snapshot_cannot_publish_old_directory_bytes(self):
        parent = self.root / 'nested'
        parent.mkdir()
        (parent / 'file').write_bytes(self.data)
        replacement = Path(self.temp.name) / 'external'
        replacement.mkdir()
        (replacement / 'file').write_bytes(b'external private data')
        original_read = os.read
        moved = False

        def replace_parent(descriptor, size):
            nonlocal moved
            if not moved:
                moved = True
                parent.rename(Path(self.temp.name) / 'detached-parent')
                parent.symlink_to(replacement, target_is_directory=True)
            return original_read(descriptor, size)

        with patch('remote_upload.os.read', side_effect=replace_parent):
            with self.assertRaises(PluginError):
                snapshot_source(self.root, 'nested/file', threading.Event())
        self.assertTrue(moved)
        self.assertEqual((replacement / 'file').read_bytes(), b'external private data')
        self.assertFalse(self.transport.calls)

    def test_cancelled_blocked_source_worker_retains_single_upload_admission(self):
        entered, release = threading.Event(), threading.Event()

        def blocked(root, source, cancel):
            entered.set()
            if not release.wait(3):
                raise AssertionError('Source worker gate was not released')
            return snapshot_source(root, source, cancel)

        with patch('remote_upload.snapshot_source', side_effect=blocked):
            try:
                flow = self.start()
                self.assertTrue(entered.wait(2))
                self.uploads.cancel({})
                self.assertTrue(self.uploads.worker_active)
                self.assertIs(self.uploads.flow, flow)
                with self.assertRaises(PluginError) as raised:
                    self.uploads.start({'invocation': 'physical:new'}, '/remote')
                self.assertEqual(raised.exception.code, 'busy')
                self.assertFalse(self.transport.calls)
            finally:
                release.set()
            self.assertTrue(flow['done'].wait(3))
        self.assertIsNone(self.uploads.flow)
        self.assertFalse(self.uploads.worker_active)
        self.assertFalse(self.transport.calls)
        self.assertIn(('job.finish', {'job': flow['job'], 'state': 'cancelled'}), self.port.calls)

    def test_stale_or_duplicate_confirmation_cannot_upload_twice(self):
        flow = self.prepare()
        frozen = flow['data']
        (self.root / 'file').write_bytes(b'changed after preparation')
        self.uploads.confirm({'invocation': 'physical:confirm'})
        surface = flow['surface']
        self.assertIsNone(self.submit('stale:surface'))
        self.assertEqual(len(self.transport.calls), 1)
        self.submit(surface)
        self.submit(surface)
        self.assertTrue(flow['done'].wait(3))
        uploads = [call for call in self.transport.calls if call[0] == 'upload']
        self.assertEqual(len(uploads), 1)
        self.assertIs(uploads[0][2], frozen)
        self.assertEqual(uploads[0][2], self.data)

    def test_unknown_snapshot_survives_cancel_and_job_events_and_refuses_fresh_upload(self):
        flow = self.prepare()
        frozen, prepared = flow['data'], flow['prepared']

        def uncertain():
            raise TransportError('unavailable', 'Reply lost', outcome_unknown=True)

        self.transport.hook = uncertain
        self.uploads.confirm({'invocation': 'physical:confirm'})
        self.submit(flow['surface'])
        self.assertTrue(flow['done'].wait(3))
        self.assertEqual(flow['phase'], 'outcome_unknown')
        with self.assertRaises(PluginError):
            self.uploads.cancel({})
        for name, data in [('job.cancel_requested', {'job': flow['job']}),
                           ('job.changed', {'job': flow['job'], 'state': 'outcome_unknown'})]:
            self.uploads.event(name, data)
        self.assertIs(self.uploads.flow, flow)
        self.assertIs(flow['data'], frozen)
        self.assertIs(flow['prepared'], prepared)
        with self.assertRaises(PluginError) as raised:
            self.uploads.start({'invocation': 'physical:new'}, '/remote')
        self.assertEqual(raised.exception.code, 'busy')
        self.assertEqual(sum(call[0] == 'upload' for call in self.transport.calls), 1)


if __name__ == '__main__':
    unittest.main()
