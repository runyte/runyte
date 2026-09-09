# SPDX-License-Identifier: MPL-2.0
"""Actual-server checks for bounded, confirmed remote namespace operations."""
import concurrent.futures
from dataclasses import FrozenInstanceError, replace
import hashlib
import os
import threading
import time
from unittest.mock import patch

from application import PluginError


class OperationTransportChecks:
    def test_namespace_mkdir_rename_and_delete_regular_file_and_empty_directory(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        prepared = transport.prepare_operation('mkdir', '猫 folder')
        self.assertFalse((fixture.root / '猫 folder').exists())
        self.assertEqual(prepared.kind, 'mkdir')
        self.assertTrue(prepared.warnings)
        with self.assertRaises(FrozenInstanceError):
            prepared.source = 'changed'
        self.assertEqual(transport.apply_operation(prepared), 'applied')
        self.assertTrue((fixture.root / '猫 folder').is_dir())
        source = fixture.root / 'notes'
        source.write_bytes(b'\x00\xffcontents')
        prepared = transport.prepare_operation('rename', 'notes', 'renamed')
        self.assertEqual(transport.apply_operation(prepared), 'applied')
        self.assertFalse(source.exists())
        self.assertEqual((fixture.root / 'renamed').read_bytes(), b'\x00\xffcontents')
        self.assertEqual(transport.apply_operation(transport.prepare_operation('delete', 'renamed')), 'applied')
        self.assertFalse((fixture.root / 'renamed').exists())
        self.assertEqual(transport.apply_operation(transport.prepare_operation('delete', '猫 folder')), 'applied')
        self.assertFalse((fixture.root / '猫 folder').exists())

    def test_namespace_rename_and_mkdir_never_replace_existing_destinations(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        source, target = fixture.root / 'source', fixture.root / 'target'
        source.write_bytes(b'source')
        target.write_bytes(b'keep destination')
        for kind, path, destination in [('mkdir', 'target', None), ('rename', 'source', 'target')]:
            with self.subTest(kind=kind), self.assertRaises(PluginError):
                transport.prepare_operation(kind, path, destination)
        self.assertEqual(source.read_bytes(), b'source')
        self.assertEqual(target.read_bytes(), b'keep destination')
        self.assertFalse(fixture.namespace_entered.is_set())
        self.assertFalse(any(op[0] in ('DELE', 'remove') and op[1] == str(target) for op in fixture.operations))

    def test_namespace_apply_rechecks_destination_created_after_preparation(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        (fixture.root / 'source').write_bytes(b'source')
        for kind, source, destination in [('mkdir', 'new-directory', None), ('rename', 'source', 'new-file')]:
            prepared = transport.prepare_operation(kind, source, destination)
            target = fixture.root / (destination or source)
            target.write_bytes(b'external creation')
            with self.subTest(kind=kind), self.assertRaises(PluginError) as raised:
                transport.apply_operation(prepared)
            self.assertEqual(raised.exception.code, 'conflict')
            self.assertEqual(target.read_bytes(), b'external creation')
        self.assertFalse(fixture.namespace_entered.is_set())

    def test_namespace_apply_hash_detects_equal_size_equal_timestamp_source_edit(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        source = fixture.root / 'source'
        for kind in ['rename', 'delete']:
            source.write_bytes(b'base')
            previous = source.stat()
            prepared = transport.prepare_operation(kind, 'source', 'renamed' if kind == 'rename' else None)
            source.write_bytes(b'edit')
            os.utime(source, ns=(previous.st_atime_ns, previous.st_mtime_ns))
            with self.subTest(kind=kind), self.assertRaises(PluginError) as raised:
                transport.apply_operation(prepared)
            self.assertEqual(raised.exception.code, 'conflict')
            self.assertEqual(source.read_bytes(), b'edit')
        self.assertFalse(fixture.namespace_entered.is_set())

    def test_namespace_nonempty_directories_and_oversized_files_are_not_traversed_or_removed(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        directory = fixture.root / 'tree'
        directory.mkdir()
        (directory / 'child').write_bytes(b'keep nested data')
        oversized = fixture.root / 'large'
        with oversized.open('wb') as stream:
            stream.truncate(8 * 1024 * 1024 + 1)
        for source in ['tree', 'large']:
            for kind in ['rename', 'delete']:
                with self.subTest(source=source, kind=kind), self.assertRaises(PluginError):
                    transport.prepare_operation(kind, source, 'new' if kind == 'rename' else None)
        self.assertEqual((directory / 'child').read_bytes(), b'keep nested data')
        self.assertTrue(oversized.exists())
        self.assertFalse(fixture.namespace_entered.is_set())

    def test_namespace_directory_gaining_an_entry_invalidates_prepared_deletion(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        directory = fixture.root / 'directory'
        directory.mkdir()
        prepared = transport.prepare_operation('delete', 'directory')
        (directory / 'new').write_bytes(b'external child')
        with self.assertRaises(PluginError):
            transport.apply_operation(prepared)
        self.assertEqual((directory / 'new').read_bytes(), b'external child')
        self.assertFalse(fixture.namespace_entered.is_set())

    def test_namespace_cancel_before_apply_and_wrong_connection_never_mutate(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        prepared = transport.prepare_operation('mkdir', 'new')
        cancel = threading.Event()
        cancel.set()
        with self.assertRaises(PluginError) as raised:
            transport.apply_operation(prepared, cancel)
        self.assertEqual(raised.exception.code, 'cancelled')
        self.assertTrue(raised.exception.settled)
        with self.assertRaises(PluginError):
            transport.apply_operation(replace(prepared, connection_id='another connection'))
        self.assertFalse((fixture.root / 'new').exists())
        self.assertFalse(fixture.namespace_entered.is_set())

    def test_namespace_paths_outside_configured_root_and_unknown_operations_are_refused(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        for kind, source, destination in [('mkdir', '../outside', None), ('rename', '../outside', 'new'),
                                           ('unknown', 'new', None), ('delete', '.', None)]:
            with self.subTest(kind=kind, source=source), self.assertRaises(PluginError):
                transport.prepare_operation(kind, source, destination)
        self.assertFalse(fixture.namespace_entered.is_set())

    def test_namespace_lost_successful_mutation_reply_is_unknown(self):
        for kind in ['mkdir', 'rename', 'delete']:
            with self.subTest(kind=kind):
                fixture = self.fixture()
                transport = self.transport(fixture)
                source = fixture.root / 'source'
                if kind != 'mkdir':
                    source.write_bytes(b'contents')
                prepared = transport.prepare_operation(kind, 'source', 'target' if kind == 'rename' else None)
                fixture.drop_namespace_reply = True
                with self.assertRaises(PluginError) as raised:
                    transport.apply_operation(prepared)
                self.assertEqual(raised.exception.code, 'outcome_unknown')
                self.assertFalse(raised.exception.settled)
                self.assertTrue(fixture.namespace_done.is_set())
                self.assertEqual(source.exists(), kind == 'mkdir')
                if kind == 'rename':
                    self.assertEqual((fixture.root / 'target').read_bytes(), b'contents')

    def test_namespace_blocked_successful_reply_times_out_as_unknown(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        prepared = transport.prepare_operation('mkdir', 'created')
        fixture.namespace_release.clear()
        with patch(transport.__class__.__module__ + '.OPERATION_TIMEOUT', 0.5):
            with self.assertRaises(PluginError) as raised:
                transport.apply_operation(prepared)
        self.assertEqual(raised.exception.code, 'outcome_unknown')
        self.assertFalse(raised.exception.settled)
        self.assertTrue((fixture.root / 'created').is_dir())

    def test_namespace_mutation_slot_survives_caller_timeout_until_actual_worker_exit(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        (fixture.root / 'notes').write_bytes(b'base')
        first = transport.prepare_operation('mkdir', 'first')
        second = transport.prepare_operation('mkdir', 'second')
        connect = transport._connect
        close_entered, release = threading.Event(), threading.Event()
        def delayed_close(operation):
            client = connect(operation)
            close = operation.client.close
            def close_client():
                close()
                if threading.current_thread().name == 'runyte-remote':
                    # Connect and mutate with the normal operation budget. Only
                    # expire after the actual remote reply, while cleanup holds
                    # the physical worker and its mutation slot alive.
                    with operation.lock:
                        operation.deadline = time.monotonic() + 0.05
                    close_entered.set()
                    release.wait(3)
            operation.client.close = close_client
            return client
        transport._connect = delayed_close
        try:
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
                future = executor.submit(transport.apply_operation, first)
                self.assertTrue(close_entered.wait(10))
                with self.assertRaises(PluginError) as raised:
                    future.result(timeout=2)
                self.assertEqual(raised.exception.code, 'outcome_unknown')
                for operation in [lambda: transport.apply_operation(second),
                                  lambda: transport.replace('notes', b'ours', hashlib.sha256(b'base').hexdigest())]:
                    with self.assertRaises(PluginError) as raised:
                        operation()
                    self.assertEqual(raised.exception.code, 'busy')
            self.assertEqual((fixture.root / 'notes').read_bytes(), b'base')
            self.assertFalse((fixture.root / 'second').exists())
        finally:
            release.set()

    def test_namespace_apply_waiting_on_hash_recheck_cancels_before_mutation(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        source = fixture.root / 'source'
        source.write_bytes(b'unchanged source')
        prepared = transport.prepare_operation('delete', 'source')
        fixture.read_started.clear()
        fixture.read_release.clear()
        cancel = threading.Event()
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            future = executor.submit(transport.apply_operation, prepared, cancel)
            try:
                self.assertTrue(fixture.read_started.wait(2))
                cancel.set()
                with self.assertRaises(PluginError) as raised:
                    future.result(timeout=2)
                self.assertEqual(raised.exception.code, 'cancelled')
                self.assertTrue(raised.exception.settled)
            finally:
                cancel.set()
                fixture.read_release.set()
        self.assertEqual(source.read_bytes(), b'unchanged source')
        self.assertFalse(fixture.namespace_entered.is_set())

    def test_document_save_reserves_the_same_mutation_slot_as_namespace_apply(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        source = fixture.root / 'notes'
        source.write_bytes(b'base')
        prepared = transport.prepare_operation('mkdir', 'new-directory')
        entered, resume = threading.Event(), threading.Event()
        def uploaded(_):
            entered.set()
            resume.wait(3)
        fixture.on_write_close = uploaded
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            future = executor.submit(transport.replace, 'notes', b'ours', hashlib.sha256(b'base').hexdigest())
            try:
                self.assertTrue(entered.wait(2))
                with self.assertRaises(PluginError) as raised:
                    transport.apply_operation(prepared)
                self.assertEqual(raised.exception.code, 'busy')
            finally:
                resume.set()
            self.assertEqual(future.result(timeout=3), hashlib.sha256(b'ours').hexdigest())
        self.assertFalse((fixture.root / 'new-directory').exists())
