# SPDX-License-Identifier: MPL-2.0
"""Real editor acceptance and synchronization regressions."""
import os
from pathlib import Path
import signal
import shlex
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock, patch

import editor_fixture as fixture
from editor_fixture import (BINARY, UNIX_PTY, NativeEditor, compact_presentation,
                            CONTROL, terminal_command)
from mcp_client import RealMCPClient
from workspace_readiness import wait_for_workspaces

if UNIX_PTY:
    from startup import Terminal


@unittest.skipUnless(UNIX_PTY, 'Unix PTY fixture')
class NativeFixtureSynchronizationTests(unittest.TestCase):
    def screen_fixture(self):
        editor = NativeEditor.__new__(NativeEditor)
        editor.lock = threading.Lock()
        editor.output = bytearray()
        editor.screen = Terminal()
        editor.errors = []
        editor.process = Mock()
        editor.process.poll.return_value = None
        return editor

    def test_readiness_requires_current_complete_screen_and_decodes_split_utf8(self):
        editor = self.screen_fixture()
        marker = 'Ready é界'

        def feed(data):
            editor.output.extend(data)
            editor.screen.feed(data)

        data = ('\x1b[?2026h' + marker).encode()
        for byte in data:
            feed(bytes([byte]))
        with self.assertRaisesRegex(AssertionError, 'incomplete frame'):
            editor.wait_output(marker, seconds=.01)
        feed(b'\x1b[?2026l')
        editor.wait_output(marker, seconds=.05)
        feed(b'\x1b[2J')
        self.assertIn(marker.encode(), editor.output)
        with self.assertRaisesRegex(AssertionError, 'did not display'):
            editor.wait_output(marker, seconds=.01)

    def test_rename_readiness_reconstructs_cells_omitted_by_incremental_redraw(self):
        editor = self.screen_fixture()
        prefix = 'terminal 1 named '
        initial = ('\x1b[1;1H' + prefix + 'eeeeeeee').encode()
        # Cells already containing 'e' are not repainted by a terminal diff.
        # The byte stream says "Dtachd" while the final screen says "Detached".
        delta = b''.join(
            f'\x1b[1;{len(prefix) + index + 1}H{char}'.encode()
            for index, char in enumerate('Detached') if char != 'e')
        for data in (initial, delta):
            editor.output.extend(data)
            editor.screen.feed(data)
        self.assertNotIn(b'namedDetached', compact_presentation(editor.output))
        editor.wait_output('namedDetached', seconds=.05, compact=True)

    def test_revocation_waits_for_applied_permissions_in_a_complete_frame(self):
        editor = self.screen_fixture()
        editor.fd = 123
        editor.command = Mock()
        editor.screen.feed(b'MCP permissions: Enabled')
        now = 0
        frames = iter([
            b'',  # The editor has not processed the queued native keys yet.
            b'\x1b[?2026h\x1b[2JNot granted',
            b'\x1b[?2026l',
        ])

        def advance(_seconds):
            nonlocal now
            now += .15  # A delayed acknowledgement exceeds both old sleeps.
            editor.screen.feed(next(frames))

        with patch('editor_fixture.os.write') as write, \
                patch('editor_fixture.time.monotonic', side_effect=lambda: now), \
                patch('editor_fixture.time.sleep', side_effect=advance):
            editor.revoke_context_access('codex')

        write.assert_called_once_with(editor.fd, b'x')
        self.assertGreater(now, .2)
        self.assertIn('Not granted', fixture.screen_text(editor.screen))

    def test_terminal_marker_is_child_output_and_absent_from_typed_command(self):
        with tempfile.TemporaryDirectory(prefix='ry-terminal-fixture-') as directory:
            stop = Path(directory) / 'stop'
            stop.touch()
            for marker in ('CLAUDE_LIVE_MARKER', "é' MARKER"):
                command = terminal_command(marker, stop)
                self.assertNotIn(marker, command)
                child = subprocess.run(shlex.split(command.removeprefix('terminal ')),
                                       capture_output=True, encoding='utf-8', timeout=5, check=True)
                self.assertEqual(child.stdout, marker + '\n')

    def test_omitted_prompt_gaps_cannot_reconstruct_a_child_marker(self):
        marker = 'CLAUDE_LIVE_MARKER'
        split = len(marker) // 2
        old_prompt = (marker[:split] + ' ' + marker[split:]).encode()
        self.assertEqual(CONTROL.sub(b'', old_prompt.replace(b' ', b'\x1b[2D')), marker.encode())
        command = terminal_command(marker, Path('/fixture/stop'))
        # Retained raw diagnostics can omit unchanged cells; the marker must
        # not appear in those diagnostics or in the reconstructed prompt.
        rendered = command.encode().replace(b' ', b'\x1b[2D')
        self.assertNotIn(marker.encode(), CONTROL.sub(b'', rendered))

    def test_compact_rename_status_survives_cursor_moved_gaps_but_command_cannot_match(self):
        marker = b'namedClaude'
        status = b'terminal 1 named\x1b[2D Claude'
        self.assertIn(marker, compact_presentation(status))
        self.assertNotIn(marker, compact_presentation(b'terminal-rename Claude'))

    def test_terminal_waits_for_child_output_and_rename_acknowledgement(self):
        with tempfile.TemporaryDirectory(prefix='ry-terminal-fixture-') as directory:
            editor = NativeEditor.__new__(NativeEditor)
            editor.project = Path(directory)
            editor.terminal_number = 0
            editor.stop_files = []
            editor.terminal_input = False
            events = []
            editor.command = lambda command: events.append(('command', command))
            editor.wait_output = lambda marker, **kwargs: events.append(
                ('output', marker, kwargs['deadline'], kwargs))
            editor.terminal('Claude', 'CLAUDE_LIVE_MARKER')
            self.assertEqual([event[:2] for event in events if event[0] == 'output'], [
                ('output', 'CLAUDE_LIVE_MARKER'),
                ('output', 'namedClaude'),
            ])
            self.assertEqual(events[1][2], events[3][2])
            self.assertNotIn('compact', events[1][3])
            self.assertTrue(events[3][3]['compact'])
            self.assertEqual([event[:2] for event in events], [
                ('command', terminal_command('CLAUDE_LIVE_MARKER', editor.stop_files[0])),
                ('output', 'CLAUDE_LIVE_MARKER'),
                ('command', 'terminal-rename Claude'),
                ('output', 'namedClaude'),
            ])


@unittest.skipUnless(UNIX_PTY and BINARY,
                     'set RUNYTE_CONTEXT_TEST_BINARY on Unix to run real editor/MCP integration')
class RealRunyteTests(fixture.EditorFixture):
    def client(self, identity):
        client = RealMCPClient(self.binary, self.root, self.env, identity)
        self.addCleanup(client.close)
        return client

    def workspaces(self, client, projects=None):
        projects = self.projects if projects is None else projects
        # The first standalone frame precedes optional host services. Wait for
        # successful live discovery and grants, not merely rendered file text.
        inventory = wait_for_workspaces(
            lambda seconds: client.data('list_workspaces', response_seconds=min(10, seconds)), projects)
        rows = inventory['workspaces']
        self.assertEqual({Path(row['root']) for row in rows}, set(projects))
        self.assertEqual(len(rows), len(projects))
        self.assertTrue(all(row['scopes'] and row['unavailable_reason'] is None for row in rows))
        return {Path(row['root']).name: row['workspace'] for row in rows}

    def test_cleanup_waits_for_a_slow_terminal_exit_before_quitting(self):
        editor = self.editor(0)
        original_command = terminal_command

        def slow_command(marker, stop):
            # The child acknowledges the stop file, then remains live beyond
            # the old 150 ms cleanup delay. This reproduces a slow reaping
            # schedule without relying on machine load or platform timing.
            return original_command(marker, stop).replace(
                'done', 'done; /bin/sleep 0.5')

        with patch('editor_fixture.terminal_command', side_effect=slow_command):
            editor.terminal('SlowExit', 'SLOW_CHILD_MARKER')
        # close is registered as cleanup: its ordinary exit must succeed.

    def test_queued_command_keys_keep_escape_separate_from_colon(self):
        editor = self.editor(0)

        def queued(command):
            # The sender's sleeps cannot establish a key boundary when the
            # reader is descheduled. Stop our own editor until every command
            # byte has been queued, then let the real Crossterm parser read it.
            editor.process.send_signal(signal.SIGSTOP)
            try:
                deadline = time.monotonic() + 5
                while True:
                    pid, status = os.waitpid(editor.process.pid, os.WUNTRACED | os.WNOHANG)
                    if pid:
                        self.assertTrue(os.WIFSTOPPED(status), 'fixture exited before pause')
                        break
                    if time.monotonic() >= deadline:
                        self.fail('fixture did not acknowledge SIGSTOP')
                    time.sleep(.01)
                editor.command(command)
            finally:
                editor.process.send_signal(signal.SIGCONT)

        stop = editor.project / 'queued-terminal-stop'
        editor.stop_files.append(stop)
        queued(terminal_command('QUEUED_CHILD_MARKER', stop))
        editor.terminal_input = True
        editor.wait_output('QUEUED_CHILD_MARKER', seconds=5)
        queued('terminal-rename Queued')
        editor.wait_output('namedQueued', seconds=5, compact=True)

        client = self.client('codex')
        workspace = self.workspaces(client, [self.projects[0]])['one']
        buffers = client.data('find_resources', kind='buffer', workspace=workspace)['results']
        original = next(row for row in buffers if row['name'].endswith('note.txt'))
        text = client.data('read_buffer', buffer=original['buffer'],
                           expected_revision=original['revision'],
                           **{'from': 0, 'to': original['chars']})['text']
        self.assertEqual(text, 'ORIGINAL_BUFFER_MARKER\n')

    def test_two_agent_clients_read_live_and_detached_workspaces_edit_unsaved_and_observe_revocation(self):
        standalone = self.editor(0)
        standalone.terminal('Claude', 'CLAUDE_LIVE_MARKER')
        standalone.command('vsplit')
        standalone.terminal('Codex', 'CODEX_LIVE_MARKER')
        persistent = self.editor(1, persistent=True)
        persistent.terminal('Detached', 'DETACHED_LIVE_MARKER')
        persistent.detach()

        codex, claude = self.client('codex'), self.client('claude')
        codex_workspaces, claude_workspaces = self.workspaces(codex), self.workspaces(claude)
        for client, workspaces, target in [(codex, codex_workspaces, 'Claude'), (claude, claude_workspaces, 'Codex')]:
            terminals = client.data('find_resources', kind='terminal', workspace=workspaces['one'])['results']
            self.assertEqual({row['name'] for row in terminals}, {'Claude', 'Codex'})
            self.assertEqual(len({row['pane'] for row in terminals}), 2)
            self.assertTrue(all(row['pane'] is not None for row in terminals))
            terminal = next(row for row in terminals if row['name'] == target)
            read = client.data('read_terminal', terminal=terminal['terminal'])
            self.assertIn(target.upper() + '_LIVE_MARKER', '\n'.join(row['text'] for row in read['rows']))
            detached = client.data('find_resources', kind='terminal', workspace=workspaces['two'])['results'][0]
            read = client.data('read_terminal', terminal=detached['terminal'])
            self.assertIn('DETACHED_LIVE_MARKER', '\n'.join(row['text'] for row in read['rows']))

        buffers = codex.data('find_resources', kind='buffer', workspace=codex_workspaces['one'])['results']
        original = next(row for row in buffers if row['name'].endswith('note.txt'))
        inserted = 'agent line one\nagent line two\n'
        edited = codex.data('edit_buffer', buffer=original['buffer'],
                           expected_revision=original['revision'], changes=[{'from': 0, 'to': 0, 'text': inserted}])
        read = codex.data('read_buffer', buffer=original['buffer'],
                         expected_revision=edited['revision'], **{'from': 0, 'to': original['chars'] + len(inserted)})
        self.assertEqual(read['text'], inserted + 'ORIGINAL_BUFFER_MARKER\n')
        self.assertEqual((self.projects[0] / 'note.txt').read_text(), 'ORIGINAL_BUFFER_MARKER\n')
        observed = next(row for row in claude.data('find_resources', kind='buffer', workspace=claude_workspaces['one'])['results']
                        if row['name'].endswith('note.txt'))
        self.assertTrue(observed['dirty'])
        self.assertEqual(claude.data('read_buffer', buffer=observed['buffer'],
            expected_revision=observed['revision'], **{'from': 0, 'to': observed['chars']})['text'], read['text'])

        # Revoke only Codex in workspace one through genuine native overlay input.
        standalone.revoke_context_access('codex')
        denied = codex.tool('read_buffer', buffer=original['buffer'],
                           expected_revision=edited['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(denied['isError'])
        self.assertEqual(claude.data('read_buffer', buffer=observed['buffer'],
            expected_revision=observed['revision'], **{'from': 0, 'to': observed['chars']})['text'], read['text'])
        final = codex.data('list_workspaces')['workspaces']
        self.assertFalse(next(row for row in final if Path(row['root']).name == 'one')['scopes'])
        self.assertTrue(next(row for row in final if Path(row['root']).name == 'two')['scopes'])

    def test_concurrent_appends_from_two_agents_land_whole_without_a_revision(self):
        editor = self.editor(0)
        codex, claude = self.client('codex'), self.client('claude')
        targets = []
        for client in (codex, claude):
            workspace = self.workspaces(client, [self.projects[0]])['one']
            buffers = client.data('find_resources', kind='buffer', workspace=workspace)['results']
            targets.append((client, workspace, next(row for row in buffers if row['name'].endswith('note.txt'))))
        errors, results = [], [[], []]

        def write(index):
            client, workspace, row = targets[index]
            try:
                for turn in range(10):
                    results[index].append(client.data('append_buffer', buffer=row['buffer'],
                                                      text=f'[{index}:{turn}] 界\n'))
            except BaseException as error:
                errors.append(error)

        threads = [threading.Thread(target=write, args=(index,)) for index in range(2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(30)
        self.assertEqual(errors, [])
        client, workspace, row = targets[0]
        observed = next(item for item in client.data('find_resources', kind='buffer', workspace=workspace)['results']
                        if item['name'].endswith('note.txt'))
        text = client.data('read_buffer', buffer=row['buffer'],
                           expected_revision=observed['revision'], **{'from': 0, 'to': observed['chars']})['text']
        self.assertTrue(text.startswith('ORIGINAL_BUFFER_MARKER\n'))
        for index in range(2):
            for turn in range(10):
                self.assertEqual(text.count(f'[{index}:{turn}] 界\n'), 1)
        ranges = sorted((item['from'], item['to']) for batch in results for item in batch)
        self.assertEqual(ranges[0][0], len('ORIGINAL_BUFFER_MARKER\n'))
        self.assertTrue(all(left[1] == right[0] for left, right in zip(ranges, ranges[1:])))
        self.assertEqual(ranges[-1][1], observed['chars'])
        self.assertTrue(all(item['preview'] == text[item['from']:item['to']] for batch in results for item in batch))
        self.assertEqual((self.projects[0] / 'note.txt').read_text(), 'ORIGINAL_BUFFER_MARKER\n')
        with self.assertRaises(AssertionError):
            codex.data('append_buffer', buffer=targets[0][2]['buffer'],
                       text='never', expected_tail='ORIGINAL_BUFFER_MARKER\n')
        self.assertNotIn('never', client.data('read_buffer', buffer=row['buffer'],
            expected_revision=observed['revision'], **{'from': 0, 'to': observed['chars']})['text'])
