# SPDX-License-Identifier: MPL-2.0
"""Actual-server binary upload checks shared by SFTP and FTP/FTPS."""
import concurrent.futures
from dataclasses import FrozenInstanceError, replace
import hashlib
import threading
import time

from application import PluginError


class UploadTransportChecks:
    def test_upload_creates_and_replaces_exact_binary_and_empty_files(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        for number, data in enumerate([b'', b'\x00\xff\xfe\r\n' + '猫'.encode(), b'x' * (128 * 1024 + 3)]):
            name = 'binary-猫-' + str(number)
            target = fixture.root / name
            prepared = transport.prepare_upload(name, data)
            self.assertIsNone(prepared.expected_version)
            self.assertEqual(prepared.bytes, len(data))
            self.assertEqual(prepared.sha256, hashlib.sha256(data).hexdigest())
            self.assertTrue(prepared.warnings)
            self.assertFalse(target.exists())
            with self.assertRaises(FrozenInstanceError):
                prepared.destination = 'different'
            progress = []
            self.assertEqual(transport.upload(prepared, data, progress=progress.append), 'applied')
            self.assertEqual(target.read_bytes(), data)
            self.assertEqual(progress[-1], 100)
            self.assertTrue(all(0 <= value <= 90 for value in progress[:-1]))
            self.assertEqual(progress, sorted(progress))
            new = b'\x00replacement\xff' if not data else b''
            prepared = transport.prepare_upload(name, new)
            self.assertEqual(prepared.expected_version, hashlib.sha256(data).hexdigest())
            self.assertEqual(transport.upload(prepared, new), 'applied')
            self.assertEqual(target.read_bytes(), new)
            self.assertFalse(any(op[0] in ('remove', 'DELE') and op[1] == str(target)
                                 for op in fixture.operations))

    def test_upload_refuses_changed_bytes_connection_and_out_of_bounds_data(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        prepared = transport.prepare_upload('new', b'confirmed')
        for candidate, data in [(prepared, b'changed'), (replace(prepared, bytes=0), b'confirmed'),
                                (replace(prepared, connection_id='another'), b'confirmed')]:
            with self.assertRaises(PluginError) as raised:
                transport.upload(candidate, data)
            self.assertEqual(raised.exception.code, 'conflict')
        for data in [bytearray(b'mutable'), 'text', b'x' * (8 * 1024 * 1024 + 1)]:
            with self.assertRaises(PluginError):
                transport.prepare_upload('new', data)
        for path in ['../escape', transport.root]:
            with self.assertRaises(PluginError):
                transport.prepare_upload(path, b'')
        self.assertFalse((fixture.root / 'new').exists())
        self.assertFalse(fixture.rename_entered.is_set())

    def test_upload_rechecks_created_deleted_and_changed_targets_before_staging(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        target = fixture.root / 'target'
        for initial, changed in [(None, b'external'), (b'base', None), (b'base', b'edit')]:
            if target.exists():
                target.unlink()
            if initial is not None:
                target.write_bytes(initial)
            prepared = transport.prepare_upload('target', b'upload')
            if changed is None:
                target.unlink()
            else:
                target.write_bytes(changed)
            with self.assertRaises(PluginError) as raised:
                transport.upload(prepared, b'upload')
            self.assertEqual(raised.exception.code, 'conflict')
            self.assertTrue(raised.exception.settled)
            self.assertEqual(target.read_bytes() if target.exists() else None, changed)
        self.assertFalse(fixture.rename_entered.is_set())
        self.assertFalse(any(path.name.startswith('.runyte-upload-') for path in fixture.root.iterdir()))

    def test_upload_last_hash_or_absence_recheck_catches_changes_during_staging(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        target = fixture.root / 'target'
        for existing in [False, True]:
            if target.exists():
                target.unlink()
            if existing:
                target.write_bytes(b'base')
            prepared = transport.prepare_upload('target', b'upload')
            fixture.on_write_close = lambda _: target.write_bytes(b'external race')
            with self.assertRaises(PluginError) as raised:
                transport.upload(prepared, b'upload')
            self.assertEqual(raised.exception.code, 'conflict')
            self.assertTrue(raised.exception.settled)
            self.assertEqual(target.read_bytes(), b'external race')
        self.assertFalse(fixture.rename_entered.is_set())

    def test_upload_cancellation_during_staging_is_settled_without_promotion(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        prepared = transport.prepare_upload('new', b'\x00upload\xff')
        cancel = threading.Event()
        fixture.on_write_close = lambda _: cancel.set()
        progress = []
        with self.assertRaises(PluginError) as raised:
            transport.upload(prepared, b'\x00upload\xff', cancel, progress.append)
        self.assertEqual(raised.exception.code, 'cancelled')
        self.assertTrue(raised.exception.settled)
        self.assertNotIn(100, progress)
        self.assertFalse((fixture.root / 'new').exists())
        self.assertFalse(fixture.rename_entered.is_set())

    def test_upload_lost_create_and_replace_promotion_replies_remain_unknown(self):
        for existing in [False, True]:
            fixture = self.fixture()
            transport = self.transport(fixture)
            target = fixture.root / 'target'
            if existing:
                target.write_bytes(b'base')
            prepared = transport.prepare_upload('target', b'\x00uploaded\xff')
            fixture.drop_rename_reply = True
            progress = []
            with self.assertRaises(PluginError) as raised:
                transport.upload(prepared, b'\x00uploaded\xff', progress=progress.append)
            self.assertEqual(raised.exception.code, 'outcome_unknown')
            self.assertFalse(raised.exception.settled)
            self.assertEqual(target.read_bytes(), b'\x00uploaded\xff')
            self.assertTrue(fixture.rename_done.is_set())
            self.assertNotIn(100, progress)
            self.assertFalse(any(op[0] in ('remove', 'DELE') and op[1] == str(target)
                                 for op in fixture.operations))

    def test_upload_timeout_retains_mutation_slot_through_actual_cleanup(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        (fixture.root / 'notes').write_bytes(b'base')
        first = transport.prepare_upload('first', b'first')
        second = transport.prepare_upload('second', b'second')
        namespace = transport.prepare_operation('mkdir', 'directory')
        connect = transport._connect
        entered, release = threading.Event(), threading.Event()

        def delayed_close(operation):
            client = connect(operation)
            close = operation.client.close
            def close_client():
                close()
                if threading.current_thread().name == 'runyte-remote':
                    with operation.lock:
                        operation.deadline = time.monotonic() + 0.05
                    entered.set()
                    release.wait(3)
            operation.client.close = close_client
            return client
        transport._connect = delayed_close
        try:
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
                future = executor.submit(transport.upload, first, b'first')
                self.assertTrue(entered.wait(10))
                with self.assertRaises(PluginError) as raised:
                    future.result(timeout=2)
                self.assertEqual(raised.exception.code, 'outcome_unknown')
                for action in [lambda: transport.upload(second, b'second'),
                               lambda: transport.apply_operation(namespace),
                               lambda: transport.replace('notes', b'edit', hashlib.sha256(b'base').hexdigest())]:
                    with self.assertRaises(PluginError) as raised:
                        action()
                    self.assertEqual(raised.exception.code, 'busy')
            self.assertEqual((fixture.root / 'notes').read_bytes(), b'base')
            self.assertFalse((fixture.root / 'second').exists())
            self.assertFalse((fixture.root / 'directory').exists())
        finally:
            release.set()
