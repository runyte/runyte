# SPDX-License-Identifier: MPL-2.0
"""Terminal-free channel wake acceptance against the real host and MCP adapter."""
import hashlib
import json
import queue
import subprocess
import threading
import unittest

import editor_fixture as fixture
from mcp_client import FRAME_BYTES, PROTOCOL, RealMCPClient


class NotificationClient(RealMCPClient):
    """Keep unsolicited events observable even while a tool response is pending."""

    def __init__(self, binary, root, env, identity, notifications=None):
        arguments = [str(binary), 'mcp', '--identity', identity, '--timeout', '2']
        if notifications is not None:
            arguments += ['--channel-notifications', notifications]
        self.process = subprocess.Popen(
            arguments, cwd=root, env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.counter = 0
        self.responses = queue.Queue(maxsize=64)
        self.notifications = queue.Queue(maxsize=64)
        self.stopped = threading.Event()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        try:
            self.initialized = self.rpc('initialize', {
                'protocolVersion': PROTOCOL, 'capabilities': {},
                'clientInfo': {'name': identity, 'version': 'acceptance'},
            })['result']
            self._send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        except BaseException:
            self.close()
            raise

    def _read(self):
        try:
            while not self.stopped.is_set():
                line = self.process.stdout.readline(FRAME_BYTES + 1)
                if not line:
                    raise AssertionError('Runyte MCP exited before its response')
                if len(line) > FRAME_BYTES or not line.endswith(b'\n'):
                    raise AssertionError('Oversized or incomplete MCP frame')
                value = json.loads(line)
                target = self.responses if 'id' in value else self.notifications
                target.put(value, timeout=1)
        except Exception as error:
            if not self.stopped.is_set():
                self.responses.put(error, timeout=1)


@unittest.skipUnless(fixture.UNIX_PTY and fixture.BINARY,
                     'set RUNYTE_CONTEXT_TEST_BINARY on Unix')
class ChannelNotifications(unittest.TestCase):
    editor = fixture.EditorFixture.editor

    def setUp(self):
        fixture.EditorFixture.setUp(self)
        claude_secret = json.loads((self.root / 'ctx' / 'identity-claude.json').read_text())
        claude_identity = hashlib.sha256(claude_secret['credential'].encode()).hexdigest()
        for path in (self.root / 'ctx').glob('grant-*.json'):
            grant = json.loads(path.read_text())
            grant['scopes'].append('agent_channel')
            if grant['identity'] == claude_identity:
                grant['scopes'].append('channel_nudge')
            path.write_text(json.dumps(grant))
        self.host_editor = self.editor(0)

    def client(self, identity, notifications=None):
        client = NotificationClient(self.binary, self.root, self.env, identity, notifications)
        self.addCleanup(client.close)
        return client

    def join(self, client, provider):
        return client.data('join_channel', workspace=str(self.projects[0]),
                           provider=provider, task='notification acceptance')['participant']

    def post(self, sender, recipient, text='private message body'):
        return sender.data('post_message', workspace=str(self.projects[0]),
                           text=text, nudge=[recipient])

    def read(self, client, **arguments):
        return client.data('read_messages', workspace=str(self.projects[0]), **arguments)

    def event(self, client):
        event = client.notifications.get(timeout=5)
        self.assertEqual(event['method'], 'notifications/claude/channel')
        self.assertNotIn('id', event)
        self.assertIsInstance(event['params']['content'], str)
        self.assertIsInstance(event['params']['meta']['workspace'], str)
        return event

    def quiet(self, client):
        with self.assertRaises(queue.Empty):
            client.notifications.get(timeout=.35)

    def test_idle_notification_without_terminal_and_only_one_until_read(self):
        receiver, sender = self.client('claude', 'claude'), self.client('codex')
        self.assertEqual(receiver.initialized['capabilities']['experimental']['claude/channel'], {})
        recipient = self.join(receiver, 'claude')
        self.assertIsNone(recipient['terminal'])
        self.join(sender, 'codex')
        self.read(receiver)
        first = self.post(sender, recipient['participant'], 'MESSAGE_BODY_MUST_STAY_IN_CHANNEL')
        self.assertEqual(first['nudges'][0]['state'], 'queued')
        event = self.event(receiver)
        self.assertNotIn('MESSAGE_BODY_MUST_STAY_IN_CHANNEL', json.dumps(event))
        second = self.post(sender, recipient['participant'])
        self.assertEqual(second['nudges'][0]['state'], 'already_pending')
        self.quiet(receiver)
        read = self.read(receiver)
        self.assertEqual(len(read['messages']), 2)
        self.assertEqual(self.post(sender, recipient['participant'])['nudges'][0]['state'], 'queued')
        self.event(receiver)

    def test_unconfigured_adapter_has_no_terminal_fallback(self):
        receiver, sender = self.client('claude'), self.client('codex')
        self.assertNotIn('claude/channel', receiver.initialized['capabilities'].get('experimental', {}))
        recipient = self.join(receiver, 'claude')
        self.join(sender, 'codex')
        posted = self.post(sender, recipient['participant'])
        self.assertEqual(posted['nudges'][0]['state'], 'unavailable')
        self.quiet(receiver)
        self.assertTrue(any(row['text'] == 'private message body'
                            for row in self.read(receiver)['messages']))

    def test_revocation_stops_notification_delivery(self):
        receiver, sender = self.client('claude', 'claude'), self.client('codex')
        recipient = self.join(receiver, 'claude')
        self.join(sender, 'codex')
        self.host_editor.revoke_context_access('claude')
        posted = self.post(sender, recipient['participant'])
        self.assertEqual(posted['nudges'][0]['state'], 'unavailable')
        self.quiet(receiver)

    def test_notification_capability_does_not_replace_permission_seven(self):
        receiver, sender = self.client('codex', 'claude'), self.client('claude')
        recipient = self.join(receiver, 'codex')
        self.join(sender, 'claude')
        posted = self.post(sender, recipient['participant'])
        self.assertEqual(posted['nudges'][0]['state'], 'unavailable')
        self.quiet(receiver)

    def test_other_workspace_wake_arrives_during_long_read(self):
        self.editor(1)
        receiver, sender = self.client('claude', 'claude'), self.client('codex')
        self.join(receiver, 'claude')
        recipient = receiver.data('join_channel', workspace=str(self.projects[1]),
                                  provider='claude', task='second workspace')['participant']
        sender.data('join_channel', workspace=str(self.projects[1]),
                    provider='codex', task='second workspace')
        self.read(receiver)
        receiver.data('read_messages', workspace=str(self.projects[1]))
        receiver.counter += 1
        receiver._send({'jsonrpc': '2.0', 'id': receiver.counter,
                        'method': 'tools/call', 'params': {
                            'name': 'read_messages', 'arguments': {
                                'workspace': str(self.projects[0]), 'wait_seconds': 4,
                            },
                        }})
        with self.assertRaises(queue.Empty):
            receiver.responses.get(timeout=.1)
        posted = sender.data('post_message', workspace=str(self.projects[1]),
                             text='SECOND_WORKSPACE_MESSAGE', nudge=[recipient['participant']])
        self.assertEqual(posted['nudges'][0]['state'], 'queued')
        self.event(receiver)
        self.assertTrue(receiver.responses.empty(), 'long read must remain pending during wake')
        reply = receiver.responses.get(timeout=6)
        self.assertEqual(reply['id'], receiver.counter)
        self.assertFalse(reply['result'].get('isError'), reply)
        self.assertEqual(reply['result']['structuredContent']['data']['messages'], [])
        messages = receiver.data('read_messages', workspace=str(self.projects[1]))['messages']
        self.assertTrue(any(row['text'] == 'SECOND_WORKSPACE_MESSAGE' for row in messages))


if __name__ == '__main__':
    unittest.main()
