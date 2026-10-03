# SPDX-License-Identifier: MPL-2.0
"""Built-in binary MCP against real editors; fixtures isolate all storage."""
import os
from pathlib import Path
import time
import unittest

from editor_fixture import BINARY, UNIX_PTY
from mcp_client import RealMCPClient
import editor_fixture as fixture


@unittest.skipUnless(UNIX_PTY and BINARY, 'set RUNYTE_CONTEXT_TEST_BINARY on Unix')
class NativeMCPTests(unittest.TestCase):
    setUp = fixture.EditorFixture.setUp
    editor = fixture.EditorFixture.editor

    def client(self, identity):
        client = RealMCPClient(self.binary, self.root, self.env, identity)
        self.addCleanup(client.close)
        return client

    def call(self, client, name, **arguments):
        result = client.tool(name, **arguments)
        self.assertFalse(result['isError'], result)
        return result['structuredContent']

    def find(self, client, **arguments):
        deadline = time.monotonic() + 10
        while True:
            found = self.call(client, 'find_resources', **arguments)
            if found['results']:
                return found
            if time.monotonic() >= deadline:
                self.fail('native MCP did not find fixture resources: ' + str(found))
            time.sleep(.02)

    def test_find_terminal_by_fuzzy_contents_propose_and_edit_unsaved_buffer(self):
        editor = self.editor(0)
        editor.terminal('shell-one', 'CLAUDE_LIVE_MARKER')
        editor.command('vsplit')
        editor.terminal('shell-two', 'CODEX_LIVE_MARKER')
        started = time.monotonic()
        client = self.client('codex')
        catalog = client.rpc('tools/list', {})['result']['tools']
        startup_ms = (time.monotonic() - started) * 1000
        self.assertEqual(len(catalog), 9)
        timings = []
        for _ in range(5):
            started = time.monotonic()
            found = self.find(client, workspace=str(self.projects[0]), query='cld_liv', kind='terminal')
            timings.append((time.monotonic() - started) * 1000)
            self.assertEqual(len(found['results']), 1)
            terminal = found['results'][0]
            self.assertEqual(terminal['name'], 'shell-one')
            self.assertEqual(terminal['matched_on'], 'content')
            self.assertIn('CLAUDE_LIVE_MARKER', terminal['excerpt'])
        proposed = self.call(client, 'propose_terminal_text', terminal=terminal['terminal'], text='hello Claude')
        self.assertEqual(proposed['data']['state'], 'pending')
        self.call(client, 'cancel_terminal_proposal', proposal=proposed['data']['proposal'])
        found = self.find(client, workspace=str(self.projects[0]), query='note', kind='buffer')
        buffer = next(row for row in found['results'] if row['name'].endswith('note.txt'))
        self.call(client, 'append_buffer', buffer=buffer['buffer'], text='NATIVE_MCP_APPEND\n')
        read = self.call(client, 'read_buffer', buffer=buffer['buffer'])['data']['text']
        self.assertEqual(read, 'ORIGINAL_BUFFER_MARKER\nNATIVE_MCP_APPEND\n')
        self.assertEqual((self.projects[0] / 'note.txt').read_text(), 'ORIGINAL_BUFFER_MARKER\n')
        print(f'Native MCP debug binary: startup+catalog={startup_ms:.1f} ms; fuzzy content discovery ms={timings}', flush=True)

    def test_permissions_can_be_revoked_and_regranted_after_startup(self):
        editor = self.editor(0)
        client = self.client('codex')
        catalog = client.rpc('tools/list', {})['result']['tools']
        found = self.find(client, kind='buffer', query='note')
        old = found['results'][0]['buffer']
        editor.revoke_context_access('codex')
        denied = client.tool('read_buffer', buffer=old)
        self.assertTrue(denied['isError'])
        os.write(editor.fd, b'34')
        for _ in range(4):
            os.write(editor.fd, b'j')
            time.sleep(.06)
        os.write(editor.fd, b'\t')
        time.sleep(.06)
        os.write(editor.fd, b'\r')
        found = self.find(client, kind='buffer', query='note')
        fresh = found['results'][0]['buffer']
        self.assertNotEqual(old, fresh)
        self.call(client, 'append_buffer', buffer=fresh, text='LATE_GRANT\n')
        self.assertEqual(client.rpc('tools/list', {})['result']['tools'], catalog)

    def test_detached_workspace_and_independent_clients_keep_exact_targets(self):
        self.editor(0)
        persistent = self.editor(1, persistent=True)
        persistent.terminal('shell', 'DETACHED_CODEX_MARKER')
        persistent.detach()
        codex, claude = self.client('codex'), self.client('claude')
        found = self.find(codex, workspace=str(self.projects[1]), query='dtx', kind='terminal')
        target = found['results'][0]
        self.assertEqual(Path(found['workspaces'][0]['root']), self.projects[1])
        read = self.call(codex, 'read_terminal', terminal=target['terminal'])
        self.assertIn('DETACHED_CODEX_MARKER', '\n'.join(r['text'] for r in read['data']['rows']))
        self.find(claude, workspace=str(self.projects[1]), kind='terminal')
        self.assertTrue(claude.tool('read_terminal', terminal=target['terminal'])['isError'])

    def test_agent_can_start_before_the_first_workspace_grant(self):
        for grant in (self.root / 'ctx').glob('grant-*.json'):
            grant.unlink()
        client = self.client('codex')
        self.assertEqual(len(client.rpc('tools/list', {})['result']['tools']), 9)
        editor = self.editor(0)
        self.assertEqual(self.call(client, 'find_resources')['results'], [])
        editor.command('mcp codex')
        editor.wait_output('Not granted')
        for _ in range(4):
            os.write(editor.fd, b'j')
            time.sleep(.06)
        os.write(editor.fd, b'\t')
        time.sleep(.06)
        os.write(editor.fd, b'\r')
        found = self.find(client, kind='buffer', query='note')
        self.assertEqual(found['identity'], 'codex')
        self.assertEqual(found['workspaces'][0]['scopes'], ['terminal_read', 'editor_context_read'])
        self.assertTrue(client.tool('append_buffer', buffer=found['results'][0]['buffer'], text='denied')['isError'])
