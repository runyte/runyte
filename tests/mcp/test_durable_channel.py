# SPDX-License-Identifier: MPL-2.0
"""Run with RUNYTE_CONTEXT_TEST_BINARY pointing at the built Runyte executable."""
import os
import sqlite3
import unittest
from durable_fixture import DurableFixture, call, post


@unittest.skipUnless(os.name != 'nt' and os.environ.get('RUNYTE_CONTEXT_TEST_BINARY'), 'requires Unix host fixture and built Runyte')
class DurableChannel(unittest.TestCase):
    def setUp(self):
        self.fixture = DurableFixture(os.environ['RUNYTE_CONTEXT_TEST_BINARY'])
        self.addCleanup(self.fixture.close)
        self.fixture.start()

    def test_real_mcp_restart_receipts_history_and_private_group(self):
        f = self.fixture
        a, b, c = (f.client(name) for name in ('a', 'b', 'c'))
        ids = [call(client, 'register', provider='test', task='durable integration')['participant'] for client in (a,b,c)]
        self.assertEqual(len(set(ids)), 3)
        public = call(a, 'create', name='public')['channel']
        for n in range(25):
            post(a, public, f'history {n}', f.project)
        call(b, 'subscribe', channel=public)
        page = call(b, 'read', channel=public)
        self.assertEqual([m['text'] for m in page['messages']], [f'history {n}' for n in range(5,25)])
        self.assertEqual(page, call(b, 'read', channel=public))
        call(b, 'acknowledge', channel=public, receipt=page['receipt'])
        self.assertEqual(call(b, 'read', channel=public)['messages'], [])
        older = call(b, 'history', channel=public, before=page['older_before'])
        self.assertEqual(len(older['messages']), 5)
        group = call(a, 'create', name='fixed', fixed=True, members=[ids[1]])['channel']
        seq, args = post(a, group, 'private marker', f.project)
        self.assertEqual(seq, call(a, 'post', **args))
        self.assertTrue(c.tool('durable_channel', action='read', channel=group)['isError'])
        receipt = call(b, 'read', channel=group)
        self.assertEqual(receipt['messages'][0]['text'], 'private marker')
        b.close()
        b = f.client('b')
        self.assertEqual(call(b, 'resume')['participant'], ids[1])
        f.stop()
        f.start()
        self.assertEqual(call(b, 'read', channel=group), receipt)
        call(b, 'acknowledge', channel=group, receipt=receipt['receipt'])
        call(b, 'acknowledge', channel=group, receipt=receipt['receipt'])
        self.assertEqual(call(a, 'history', channel=group)['messages'], [])
        self.assertEqual(seq, call(a, 'post', **args))
        call(c, 'subscribe', channel=public)
        self.assertEqual(len(call(c, 'read', channel=public)['messages']), 20)
        with sqlite3.connect(f.store / 'agent-channels.sqlite3') as db:
            self.assertEqual(db.execute('SELECT count(*) FROM messages').fetchone()[0], 25)
            self.assertEqual(db.execute('PRAGMA integrity_check').fetchone()[0], 'ok')

    def test_profile_lock_scope_and_strict_actions(self):
        f = self.fixture
        a = f.client('a')
        call(a, 'register', provider='test', task='boundaries')
        with self.assertRaises(AssertionError):
            f.client('a')
        self.assertTrue(a.tool('durable_channel', action='resume', text='unexpected')['isError'])
        self.assertTrue(a.tool('durable_channel', action='read', channel='x', limit=0)['isError'])
        f.add_identity('ungranted', ())
        denied = f.client('denied', identity='ungranted')
        self.assertTrue(denied.tool('durable_channel', action='register', provider='test', task='denied')['isError'])


if __name__ == '__main__':
    unittest.main()
