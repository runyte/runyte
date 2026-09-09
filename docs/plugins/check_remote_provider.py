# SPDX-License-Identifier: MPL-2.0
"""Deterministic public provider-handler lifecycle and byte-boundary checks."""
import threading
import unittest
from application import PluginError
from remote_provider import RemoteProvider, MAX_BYTES, CHUNK_BYTES, version


class Failure(PluginError):
    def __init__(self, code, settled=False):
        super().__init__(code, 'Controlled transport failure')
        self.settled = settled


class Transport:
    connection_id = 'connection-identity'
    label = 'Fixture'

    def __init__(self):
        self.data = b'base'
        self.writes = []
        self.failure = None
        self.observe_hook = None

    def observe(self, path, cancel=None):
        if self.observe_hook:
            self.observe_hook(cancel)
        return '/notes.txt', self.data

    def replace(self, path, data, expected, cancel):
        if self.failure:
            raise self.failure
        if version(self.data) != expected:
            raise Failure('conflict', settled=True)
        if cancel.is_set():
            raise Failure('cancelled', settled=True)
        self.writes.append((path, data))
        self.data = data
        return version(data)


class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.transport = Transport()
        self.now = [0]
        self.provider = RemoteProvider(self.transport, clock=lambda: self.now[0])

    def request(self, job='read', **values):
        return {'job': job, 'provider': 'remote', 'key': self.provider.requested_key('/notes.txt'), **values}

    def begin(self, text=b'new', job='write'):
        result = self.provider.begin(self.request(job, bytes=len(text), encoding='utf-8',
            mode='confirmed_best_effort', expected_version=version(self.transport.data)))
        return {'job': job, 'upload': result['value']['upload']}

    def commit(self, text=b'new', job='write'):
        expected = version(self.transport.data)
        upload = self.begin(text, job)
        self.provider.chunk({**upload, 'offset': 0, 'text': text.decode('utf-8')})
        return self.provider.commit({**upload, 'expected_version': expected, 'mode': 'confirmed_best_effort'})

    def assert_code(self, code, callback):
        with self.assertRaises(PluginError) as raised:
            callback()
        self.assertEqual(raised.exception.code, code)

    def test_snapshot_unicode_chunks_remain_immutable_and_release_at_eof(self):
        original = (b'x' * (CHUNK_BYTES - 1)) + '猫\r\né'.encode()
        self.transport.data = original
        metadata = self.provider.stat(self.request())['value']
        self.transport.data = b'changed outside'
        offset, collected = 0, b''
        while True:
            chunk = self.provider.read(self.request(version=metadata['version'], offset=offset, limit=CHUNK_BYTES))['value']
            part = chunk['text'].encode()
            collected += part
            offset += len(part)
            if chunk['eof']:
                break
        self.assertEqual(collected, original)
        self.assertFalse(self.provider.reads)
        self.assert_code('stale', lambda: self.provider.read(self.request(version=metadata['version'], offset=0, limit=1)))

    def test_empty_and_binary_size_identity_and_byte_boundary_refusals(self):
        self.transport.data = b''
        meta = self.provider.stat(self.request('empty'))['value']
        self.assertTrue(self.provider.read(self.request('empty', version=meta['version'], offset=0, limit=1))['value']['eof'])
        for number, data in enumerate([b'\xff', b'x\0', b'x' * (MAX_BYTES + 1)]):
            self.transport.data = data
            self.assert_code('limit_exceeded' if number == 2 else 'unsupported', lambda: self.provider.stat(self.request(f'bad{number}')))
            self.assertFalse(self.provider.reads)
        self.assert_code('conflict', lambda: self.provider.stat(self.request(key='different-connection:/notes.txt')))
        self.transport.data = '猫'.encode()
        meta = self.provider.stat(self.request('scalar'))['value']
        for offset, limit in [(1, 3), (0, 1), (True, 3), (0, CHUNK_BYTES + 1)]:
            self.assert_code('invalid_argument', lambda: self.provider.read(self.request('scalar', version=meta['version'], offset=offset, limit=limit)))
        self.assert_code('stale', lambda: self.provider.read(self.request('scalar', version='changed', offset=0, limit=3)))

    def test_release_event_reclaims_cache_and_overtakes_queued_stat(self):
        self.provider.stat(self.request('one'))
        self.provider.stat(self.request('two'))
        self.assert_code('busy', lambda: self.provider.stat(self.request('three')))
        self.provider.on_event('resource.released', {'job': 'one'})
        self.provider.stat(self.request('three'))
        self.provider.on_event('resource.released', {'job': 'future'})
        self.assert_code('conflict', lambda: self.provider.stat(self.request('future')))
        self.assertEqual(len(self.provider.reads), 2)
        self.now[0] = 61
        self.provider.stat(self.request('fresh'))
        self.assertEqual(list(self.provider.reads), ['fresh'])

    def test_release_cancels_inflight_observation_without_late_cache_publication(self):
        started, resume = threading.Event(), threading.Event()
        errors = []
        def observing(cancel):
            started.set()
            self.assertTrue(resume.wait(2))
            self.assertTrue(cancel.is_set())
        self.transport.observe_hook = observing
        def read():
            try:
                self.provider.stat(self.request())
            except PluginError as error:
                errors.append(error.code)
        worker = threading.Thread(target=read)
        worker.start()
        self.assertTrue(started.wait(2))
        self.provider.on_event('resource.released', {'job': 'read'})
        resume.set()
        worker.join(2)
        self.assertFalse(worker.is_alive())
        self.assertEqual(errors, ['cancelled'])
        self.assertFalse(self.provider.reads)

    def test_staging_enforces_exact_tokens_offsets_modes_and_abort_tombstones(self):
        upload = self.begin()
        self.assertFalse(self.transport.writes)
        forged = {**upload, 'upload': 'forged:' + upload['job']}
        self.assert_code('not_found', lambda: self.provider.chunk({**forged, 'offset': 0, 'text': 'new'}))
        self.assert_code('not_found', lambda: self.provider.abort(forged))
        self.assertFalse(self.provider.uploads['write'].cancelled.is_set())
        self.assert_code('invalid_argument', lambda: self.provider.chunk({**upload, 'offset': 1, 'text': 'new'}))
        self.assert_code('invalid_argument', lambda: self.provider.chunk({**upload, 'offset': 0, 'text': 'too long'}))
        self.assert_code('conflict', lambda: self.provider.commit({**upload, 'mode': 'conditional', 'expected_version': version(b'base')}))
        self.provider.abort(upload)
        self.assertFalse(self.provider.uploads)
        self.assert_code('conflict', lambda: self.begin(job='write'))
        self.provider.abort({'job': 'late', 'upload': None})
        self.assert_code('conflict', lambda: self.begin(job='late'))
        self.assert_code('unsupported', lambda: self.provider.begin(self.request('wrong', bytes=0, encoding='utf-8', mode='conditional', expected_version=version(b'base'))))
        self.assertFalse(self.transport.writes)

    def test_confirmed_upload_and_authoritative_settlement_reconciliation(self):
        result = self.commit('猫\r\n'.encode())
        self.assertEqual(result, {'kind': 'write_committed', 'value': {'version': version('猫\r\n'.encode())}})
        self.assertFalse(self.provider.uploads)
        result = self.provider.reconcile(self.request('recover', previous_write='write'))
        self.assertEqual(result['kind'], 'reconciled')
        self.assertEqual(result['value']['previous_write'], 'write')
        self.assertEqual(result['value']['metadata']['version'], version(self.transport.data))

    def test_known_rejection_settles_but_unknown_failure_and_restart_cannot_prove_completion(self):
        for code, settled in [('conflict', True), ('cancelled', True), ('outcome_unknown', False), ('unavailable', False)]:
            with self.subTest(code=code):
                self.setUp()
                self.transport.failure = Failure(code, settled)
                result = self.commit()
                error = result['value']['error']
                self.assertEqual(error['code'], code if settled else 'outcome_unknown')
                self.assertFalse(self.provider.uploads)
                if settled:
                    self.provider.reconcile(self.request('recover', previous_write='write'))
                else:
                    self.assert_code('outcome_unknown', lambda: self.provider.reconcile(self.request('recover', previous_write='write')))
                    self.assert_code('outcome_unknown', lambda: self.provider.abort({'job': 'write', 'upload': None}))
        fresh = RemoteProvider(self.transport)
        self.assert_code('outcome_unknown', lambda: fresh.reconcile(self.request('recover', previous_write='write')))

    def test_abort_and_read_release_cannot_fabricate_remote_settlement_proof(self):
        self.commit()
        self.assert_code('outcome_unknown', lambda: self.provider.abort({'job': 'write', 'upload': None}))
        self.provider.on_event('resource.released', {'job': 'write'})
        self.provider.reconcile(self.request('known', previous_write='write'))
        self.transport.failure = Failure('outcome_unknown')
        self.commit(job='uncertain')
        self.provider.on_event('resource.released', {'job': 'uncertain'})
        self.assert_code('outcome_unknown', lambda: self.provider.reconcile(self.request('unknown', previous_write='uncertain')))
        for number in range(150):
            self.provider.abort({'job': f'evict{number}', 'upload': None})
        self.provider.abort({'job': 'uncertain', 'upload': None})
        self.assert_code('outcome_unknown', lambda: self.provider.reconcile(self.request('forgotten', previous_write='uncertain')))
        self.provider.abort({'job': 'unseen', 'upload': None})
        self.assert_code('outcome_unknown', lambda: self.provider.reconcile(self.request('unseen-read', previous_write='unseen')))

    def test_remote_filename_supplies_only_an_explicit_syntax_hint(self):
        self.transport.observe = lambda path, cancel=None: ('/notes.rs', b'fn main() {}')
        metadata = self.provider.stat(self.request())['value']
        self.assertEqual(metadata['syntax_hint'], 'rust')
        self.assertEqual(metadata['key'], 'connection-identity:/notes.rs')
        self.assertEqual(metadata['label'], 'Fixture · notes.rs')

    def test_upload_slots_expiry_and_history_remain_bounded(self):
        self.begin(job='one')
        self.begin(job='two')
        self.assert_code('busy', lambda: self.begin(job='three'))
        self.now[0] = 61
        self.begin(job='three')
        self.assertEqual(list(self.provider.uploads), ['three'])
        for number in range(150):
            self.provider.abort({'job': f'cancel{number}', 'upload': None})
        self.assertEqual(len(self.provider.settled), 128)


if __name__ == '__main__':
    unittest.main(verbosity=2)
