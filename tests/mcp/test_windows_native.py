# SPDX-License-Identifier: MPL-2.0
"""Built-in MCP against public Runyte, driven by the Rust ConPTY fixture."""
import os
from pathlib import Path
import time
import unittest

from mcp_client import RealMCPClient


class PublicWindowsMCPTests(unittest.TestCase):
    def wait_file(self, path, message):
        deadline = time.monotonic() + 30
        while not path.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertTrue(path.exists(), message)

    def client(self):
        client = RealMCPClient(self.binary, self.root, os.environ.copy(), 'agent')
        self.addCleanup(client.close)
        return client

    def find(self, client):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            found = client.data('find_resources', kind='buffer', query='unicode note.txt',
                                match_in='name')
            if found['results']:
                self.assertEqual(len(found['results']), 1)
                return found['results'][0]
            time.sleep(.05)
        self.fail(f'public Runyte buffer did not become readable: {found!r}')

    def test_public_executable_discovery_edit_reconnect_remember_restart_and_revoke(self):
        # Missing fixture variables fail this test; the Windows CI driver must
        # launch the real ConPTY scenario, never a successful all-skipped suite.
        self.binary = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_BINARY'])
        self.root = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_ROOT'])
        phase_ready = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_PHASE_READY'])
        phase_continue = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_PHASE_CONTINUE'])
        restart_ready = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_RESTART_READY'])
        revoke_continue = Path(os.environ['RUNYTE_CONTEXT_PUBLIC_REVOKE_CONTINUE'])

        first = self.client()
        self.assertEqual(len(first.rpc('tools/list', {})['result']['tools']), 9)
        buffer = self.find(first)
        self.assertEqual(first.data('read_buffer', buffer=buffer['buffer'])['text'], 'aç界🙂z')
        first.data('edit_buffer', buffer=buffer['buffer'], expected_revision=buffer['revision'],
                   changes=[{'from': 1, 'to': 4, 'text': 'Żółć🙂'}])
        stale = first.tool('read_buffer', buffer=buffer['buffer'],
                           expected_revision=buffer['revision'])
        self.assertTrue(stale['isError'])
        self.assertEqual(stale['structuredContent']['error']['code'], 'stale')
        first.close()

        second = self.client()
        current = self.find(second)
        crossed = second.tool('read_buffer', buffer=buffer['buffer'])
        self.assertTrue(crossed['isError'])
        self.assertEqual(crossed['structuredContent']['error']['code'], 'stale')
        self.assertEqual(second.data('read_buffer', buffer=current['buffer'])['text'], 'aŻółć🙂z')

        phase_ready.write_text('ready', encoding='ascii')
        self.wait_file(phase_continue, 'Rust fixture did not restart the remembered grant')
        current = self.find(second)
        self.assertNotEqual(current['buffer'], buffer['buffer'])
        # The unsaved edit disappeared, but the remembered grant survived.
        self.assertEqual(second.data('read_buffer', buffer=current['buffer'])['text'], 'aç界🙂z')

        restart_ready.write_text('ready', encoding='ascii')
        self.wait_file(revoke_continue, 'Rust fixture did not revoke the remembered grant')
        self.assertTrue(second.tool('read_buffer', buffer=current['buffer'])['isError'])
        self.assertEqual(second.data('list_workspaces')['workspaces'], [])
        self.assertEqual(second.data('find_resources')['results'], [])


if __name__ == '__main__':
    unittest.main()
