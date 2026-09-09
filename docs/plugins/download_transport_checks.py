# SPDX-License-Identifier: MPL-2.0
"""Shared actual-server binary download checks for the remote adapters."""
import hashlib
import os
import threading

from application import PluginError


class DownloadTransportChecks:
    def test_binary_download_streams_exact_raw_bytes_into_existing_staging_file(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        contents = bytes(range(256)) * 1100 + b'\x00\xff\r\n'
        source, stage = fixture.root / 'binary-download', fixture.base / 'stage'
        source.write_bytes(contents)
        stage.touch()
        stage.chmod(0o600)
        progress = []
        digest = transport.download(source.name, str(stage), len(contents), progress=progress.append)
        self.assertEqual(stage.read_bytes(), contents)
        self.assertEqual(digest, hashlib.sha256(contents).hexdigest())
        self.assertGreater(len(progress), 1)
        self.assertEqual(progress[-1], len(contents))
        self.assertEqual(progress, sorted(set(progress)))
        source.write_bytes(b'')
        empty = fixture.base / 'empty-stage'
        empty.touch()
        empty.chmod(0o600)
        self.assertEqual(transport.download(source.name, str(empty), 0), hashlib.sha256(b'').hexdigest())

    def test_download_rejects_declared_size_changes_and_invalid_staging_targets(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        source, target = fixture.root / 'download', fixture.base / 'existing'
        source.write_bytes(b'remote bytes')
        target.write_bytes(b'keep local bytes')
        link = fixture.base / 'stage-link'
        link.symlink_to(target)
        for path in [target, link, fixture.base / 'missing']:
            with self.subTest(path=path.name), self.assertRaises(PluginError):
                transport.download(source.name, str(path), len(b'remote bytes'))
        self.assertEqual(target.read_bytes(), b'keep local bytes')
        empty_target = fixture.base / 'empty-target'
        empty_target.touch()
        empty_target.chmod(0o600)
        hard_link = fixture.base / 'hard-link'
        os.link(empty_target, hard_link)
        public_stage = fixture.base / 'public-stage'
        public_stage.touch()
        public_stage.chmod(0o644)
        for path in [hard_link, public_stage]:
            with self.subTest(path=path.name), self.assertRaises(PluginError):
                transport.download(source.name, str(path), len(b'remote bytes'))
        self.assertEqual(empty_target.read_bytes(), b'')
        for size in [0, 30, 8 * 1024 * 1024 + 1]:
            stage = fixture.base / f'stage{size}'
            stage.touch()
            stage.chmod(0o600)
            with self.subTest(size=size), self.assertRaises(PluginError):
                transport.download(source.name, str(stage), size)

    def test_binary_download_cancellation_never_modifies_remote_source(self):
        fixture = self.fixture()
        transport = self.transport(fixture)
        source, stage = fixture.root / 'cancel-download', fixture.base / 'stage'
        contents = b'\x00\xff' * 200000
        source.write_bytes(contents)
        stage.touch()
        stage.chmod(0o600)
        cancel = threading.Event()
        with self.assertRaises(PluginError) as raised:
            transport.download(source.name, str(stage), len(contents), cancel,
                               progress=lambda _: cancel.set())
        self.assertEqual(raised.exception.code, 'cancelled')
        self.assertTrue(raised.exception.settled)
        self.assertEqual(source.read_bytes(), contents)
        self.assertLess(stage.stat().st_size, len(contents))
