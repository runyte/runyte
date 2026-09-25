# SPDX-License-Identifier: MPL-2.0
"""Optional real Runyte + two real stdio MCP clients; no accounts or network.

RUNYTE_CONTEXT_TEST_BINARY must name a prebuilt editor. Fixture-owned grants
are seeded before launch; revocation uses physical input in the native overlay.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import shlex
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock

from test_bridge import MCPClient, PACKAGE, REPO
from workspace_readiness import wait_for_workspaces
from runyte_context.client import FRAME_BYTES, decode, encode
from runyte_context.server import PROTOCOL

UNIX_PTY = os.name != 'nt'
if UNIX_PTY:
    from native_pty import spawn as spawn_pty

    sys.path.insert(0, str(REPO / 'benchmarks'))
    import ptybench
    from startup import Terminal

BINARY = os.environ.get('RUNYTE_CONTEXT_TEST_BINARY')
SCOPES = ['terminal_read', 'editor_context_read', 'buffer_edit', 'terminal_propose']
CONTROL = re.compile(rb'\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|P[^\x1b]*\x1b\\)')


def compact_presentation(value):
    return b''.join(CONTROL.sub(b'', value).split())


def screen_text(terminal, compact=False):
    # A synchronized update is not visible until its closing sequence. Read
    # cells directly: pyte's display helper mishandles some wide glyphs.
    if (2026 << 5) in terminal.screen.mode:
        return None
    screen = terminal.screen
    text = '\n'.join(''.join(screen.buffer[row][column].data
                            for column in range(screen.columns))
                     for row in range(screen.lines))
    return ''.join(text.split()) if compact else text


def private_json(path, value):
    with open(path, 'x', opener=lambda name, flags: os.open(name, flags, 0o600)) as target:
        json.dump(value, target)


def seed_identity(root, name, projects):
    credential = secrets.token_hex(32)
    identity = hashlib.sha256(credential.encode()).hexdigest()
    private_json(root / ('identity-' + name + '.json'), {'name': name, 'credential': credential})
    for project in projects:
        path = os.fsencode(project.resolve())
        key = hashlib.sha256(path + b'\0' + identity.encode()).hexdigest()
        private_json(root / ('grant-' + key + '.json'), {'root': list(path), 'identity': identity, 'scopes': SCOPES})


def terminal_command(marker, stop):
    # The prompt renders the command before the child starts. Forced adjacent
    # quotes keep this argv short while leaving shell syntax between marker
    # halves even when terminal cursor movement omits blank cells.
    def quoted(value):
        return "'" + value.replace("'", "'\"'\"'") + "'"

    split = len(marker) // 2
    assert 0 < split < len(marker)
    marker_word = quoted(marker[:split]) + quoted(marker[split:])
    script = 'printf "%s\\n" "$1"; while [ ! -f "$2" ]; do /bin/sleep 0.05; done'
    prefix = ['/bin/sh', '-c', script, 'context-fixture']
    command = ('terminal ' + ' '.join(shlex.quote(value) for value in prefix)
               + ' ' + marker_word + ' ' + shlex.quote(str(stop)))
    assert marker not in command
    return command


class NativeEditor:
    def __init__(self, binary, project, config, env, persistent=False):
        self.binary, self.project, self.config, self.env = binary, project, config, env
        self.persistent = persistent
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.output = bytearray()
        self.screen = Terminal()
        self.reaped = False
        self.stop_files = []
        self.terminal_number = 0
        self.terminal_input = False
        self.errors = []
        self.host = None
        if persistent:
            self.host = subprocess.Popen([str(binary), '--serve', '--project-root', str(project),
                '--config', str(config)], env=env, cwd=project,
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 15
                while True:
                    if self.host.poll() is not None:
                        raise AssertionError('Fixture persistent host exited before registration')
                    records = Path(env['RUNYTE_CONTEXT_HOME']).glob('host-*.json')
                    if any(json.loads(path.read_text())['pid'] == self.host.pid for path in records):
                        break
                    if time.monotonic() >= deadline:
                        raise AssertionError('Fixture persistent host registration exceeded deadline')
                    time.sleep(.02)
            except BaseException:
                self.host.kill()
                self.host.wait(timeout=5)
                raise
        arguments = [str(binary), '--persistent' if persistent else '--standalone',
                     '--project-root', str(project), '--config', str(config)]
        if not persistent:
            arguments.append('note.txt')
        self.process = self.fd = None
        try:
            self.process, self.fd = spawn_pty(
                arguments, cwd=project, env={**env, 'TERM': 'xterm-256color'},
                configure=ptybench._configure,
            )
            self.pump = threading.Thread(target=self.drain, daemon=True)
            self.pump.start()
        except BaseException:
            self.reap()
            if self.fd is not None:
                os.close(self.fd)
            if self.host is not None:
                self.host.kill()
                self.host.wait(timeout=5)
            raise
        try:
            self.wait_output('[about]' if persistent else 'ORIGINAL_BUFFER_MARKER')
            if persistent:
                self.command('open note.txt')
                self.wait_output('ORIGINAL_BUFFER_MARKER')
        except BaseException:
            try:
                self.close()
            except Exception:
                pass
            raise

    def drain(self):
        try:
            with selectors.DefaultSelector() as poll:
                poll.register(self.fd, selectors.EVENT_READ)
                while not self.stop.is_set():
                    if not poll.select(.05):
                        continue
                    try:
                        data = os.read(self.fd, 65536)
                    except OSError:
                        return  # Linux returns EIO after the last PTY slave closes.
                    if not data:
                        return
                    with self.lock:
                        self.output.extend(data)
                        del self.output[:-1024 * 1024]
                        reply = self.screen.feed(data)
                    if reply:
                        os.write(self.fd, reply)
        except Exception as error:
            self.errors.append(type(error).__name__)

    def wait_output(self, marker, seconds=15, *, deadline=None, compact=False):
        deadline = time.monotonic() + seconds if deadline is None else deadline
        while time.monotonic() < deadline:
            with self.lock:
                rendered = screen_text(self.screen, compact)
                if rendered is not None and marker in rendered:
                    return
            if self.errors:
                raise AssertionError('Native PTY reader failed: ' + self.errors[0])
            if self.process.poll() is not None:
                self.reaped = True
                raise self.missing_marker(marker)
            time.sleep(.02)
        raise self.missing_marker(marker)

    def missing_marker(self, marker):
        with self.lock:
            output = CONTROL.sub(b'', bytes(self.output)).decode('utf-8', 'replace')[-2048:]
            size = len(self.output)
            rendered = screen_text(self.screen)
            visible = rendered[-2048:] if rendered is not None else '<incomplete frame>'
        return AssertionError(
            f'Native editor did not display {marker!r}; '
            f'exit={self.process.poll()}, output_bytes={size}, screen_tail={visible!r}, tail={output!r}'
        )

    def command(self, command):
        prefix = b''
        if self.terminal_input:
            prefix = b'\x1c'
            self.terminal_input = False
        # A complete CSI-u Escape report cannot merge with ':' into Alt-:
        # when the editor is descheduled. Sender-side sleeps cannot establish
        # that boundary. Keep terminal exit before Escape, which belongs to
        # the child until Ctrl-\ switches back to the editor.
        os.write(self.fd, prefix + b'\x1b[27u:' + command.encode() + b'\r')

    def terminal(self, name, marker):
        deadline = time.monotonic() + 15
        self.terminal_number += 1
        stop = self.project / ('terminal-stop-' + str(self.terminal_number))
        self.stop_files.append(stop)
        # A data file ends the checked system shell. Nothing executable is written.
        self.command(terminal_command(marker, stop))
        self.terminal_input = True
        self.wait_output(marker, deadline=deadline)
        rename_command = 'terminal-rename ' + name
        rename_marker = 'named' + name
        assert rename_marker.encode() not in compact_presentation(rename_command.encode())
        self.command(rename_command)
        self.wait_output(rename_marker, deadline=deadline, compact=True)

    def detach(self):
        self.command('detach')
        self.wait_exit()

    def wait_exit(self, seconds=10):
        try:
            status = self.process.wait(timeout=seconds)
        except subprocess.TimeoutExpired as error:
            raise AssertionError('Native editor exit exceeded deadline') from error
        self.reaped = True
        if status:
            raise AssertionError(f'Native editor exited unsuccessfully: {status}')

    def reap(self):
        if self.process is not None:
            if self.process.poll() is None:
                self.process.kill()
            self.process.wait(timeout=5)
            self.reaped = True

    def close(self):
        for stop in self.stop_files:
            stop.touch()
        time.sleep(.15)
        failure = None
        try:
            if self.persistent:
                stopped = subprocess.run([str(self.binary), '--session-stop', '--force',
                    str(self.project), '--config', str(self.config)], env=self.env,
                    cwd=self.project, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
                if stopped.returncode:
                    failure = AssertionError('Fixture persistent host did not stop')
            elif not self.reaped:
                self.command('qa!')
            if not self.reaped:
                self.wait_exit()
        except Exception as error:
            failure = error
        finally:
            if self.host is not None:
                try:
                    self.host.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.host.kill()
                    self.host.wait(timeout=5)
            self.stop.set()
            if not self.reaped:
                self.reap()
            self.pump.join(1)
            os.close(self.fd)
        if failure:
            raise failure


class RealMCPClient(MCPClient):
    def __init__(self, binary, root, env, identity):
        self.process = subprocess.Popen([sys.executable, '-m', 'runyte_context', '--identity', identity,
            '--runyte', str(binary), '--timeout', '2'], cwd=root, bufsize=0,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env={**env, 'PYTHONPATH': str(PACKAGE), 'PYTHONNOUSERSITE': '1'})
        self.counter = 0
        self.notifications = []
        self.buffer = bytearray()
        try:
            self.rpc('initialize', {'protocolVersion': PROTOCOL, 'capabilities': {},
                                   'clientInfo': {'name': identity, 'version': 'integration'}})
            self.process.stdin.write(encode({'jsonrpc': '2.0', 'method': 'notifications/initialized'}))
            self.process.stdin.flush()
        except BaseException:
            self.close()
            raise

    def rpc(self, method, params, *, seconds=10):
        self.counter += 1
        self.process.stdin.write(encode({'jsonrpc': '2.0', 'id': self.counter, 'method': method, 'params': params}))
        self.process.stdin.flush()
        deadline = time.monotonic() + seconds
        with selectors.DefaultSelector() as poll:
            poll.register(self.process.stdout, selectors.EVENT_READ)
            while True:
                while b'\n' in self.buffer:
                    line, _, remaining = self.buffer.partition(b'\n')
                    self.buffer = bytearray(remaining)
                    response = decode(line)
                    if 'id' in response:
                        self.assert_id(response)
                        return response
                    self.notifications.append(response)
                    if len(self.notifications) > 64:
                        raise AssertionError('Too many MCP notifications')
                if len(self.buffer) > FRAME_BYTES:
                    raise AssertionError('Oversized MCP response')
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not poll.select(remaining):
                    raise AssertionError('Real MCP response deadline exceeded')
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise AssertionError('Real MCP bridge exited early')
                self.buffer.extend(chunk)

    def data(self, name, *, response_seconds=10, **arguments):
        result = self.rpc('tools/call', {'name': name, 'arguments': arguments},
                          seconds=response_seconds)['result']
        if result.get('isError'):
            raise AssertionError('Real MCP tool failed: ' + name)
        structured = result['structuredContent']
        return structured if name == 'list_workspaces' else structured['data']


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
                     'set RUNYTE_CONTEXT_TEST_BINARY on Unix to run real editor/bridge integration')
class RealRunyteTests(unittest.TestCase):
    def setUp(self):
        self.binary = Path(BINARY).resolve(strict=True)
        self.temporary = tempfile.TemporaryDirectory(prefix='ry-mcp-', dir='/tmp')
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.env = dict(os.environ)
        for key in ('RUNYTE_PARENT_CONTEXT', 'RUNYTE_BENCH_EVENTS', 'RUNYTE_CONTEXT_TEST_INVENTORY', 'RUNYTE_INPUT_TRACE'):
            self.env.pop(key, None)
        for key, directory in [('HOME', 'home'), ('XDG_CONFIG_HOME', 'config'), ('XDG_CACHE_HOME', 'cache'),
                ('XDG_RUNTIME_DIR', 'runtime'), ('XDG_STATE_HOME', 'state'), ('XDG_DATA_HOME', 'data'),
                ('RUNYTE_ALL_HOSTS_DIR', 'hosts'), ('RUNYTE_CONTEXT_HOME', 'ctx')]:
            target = self.root / directory
            target.mkdir(mode=0o700)
            self.env[key] = str(target)
        self.projects = [self.root / name for name in ('one', 'two')]
        for project in self.projects:
            project.mkdir()
            (project / 'note.txt').write_text('ORIGINAL_BUFFER_MARKER\n')
        for identity in ('codex', 'claude'):
            seed_identity(self.root / 'ctx', identity, self.projects)
        self.config = self.root / 'config' / 'config.yaml'
        self.config.write_text('lsp:\n  enable: false\nworkspace:\n  session_strip: hidden\n')

    def editor(self, index, persistent=False):
        editor = NativeEditor(self.binary, self.projects[index], self.config, self.env, persistent)
        self.addCleanup(editor.close)
        return editor

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
        self.assertTrue(all(row['readable'] for row in rows))
        return {Path(row['root']).name: row['workspace'] for row in rows}

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
        buffers = client.data('list_buffers', workspace=workspace)['buffers']
        original = next(row for row in buffers if row['name'].endswith('note.txt'))
        text = client.data('read_buffer', workspace=workspace, buffer=original['buffer'],
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
            terminals = client.data('list_terminals', workspace=workspaces['one'])['terminals']
            self.assertEqual({row['name'] for row in terminals}, {'Claude', 'Codex'})
            self.assertEqual(len({row['pane'] for row in terminals}), 2)
            self.assertTrue(all(row['pane'] is not None for row in terminals))
            terminal = next(row for row in terminals if row['name'] == target)
            read = client.data('read_terminal', workspace=workspaces['one'], terminal=terminal['terminal'])
            self.assertIn(target.upper() + '_LIVE_MARKER', '\n'.join(row['text'] for row in read['rows']))
            detached = client.data('list_terminals', workspace=workspaces['two'])['terminals'][0]
            read = client.data('read_terminal', workspace=workspaces['two'], terminal=detached['terminal'])
            self.assertIn('DETACHED_LIVE_MARKER', '\n'.join(row['text'] for row in read['rows']))
            panes = client.data('list_panes', workspace=workspaces['two'])
            self.assertFalse(panes['attached'])

        buffers = codex.data('list_buffers', workspace=codex_workspaces['one'])['buffers']
        original = next(row for row in buffers if row['name'].endswith('note.txt'))
        inserted = 'agent line one\nagent line two\n'
        edited = codex.data('edit_buffer', workspace=codex_workspaces['one'], buffer=original['buffer'],
                           expected_revision=original['revision'], changes=[{'from': 0, 'to': 0, 'text': inserted}])
        read = codex.data('read_buffer', workspace=codex_workspaces['one'], buffer=original['buffer'],
                         expected_revision=edited['revision'], **{'from': 0, 'to': original['chars'] + len(inserted)})
        self.assertEqual(read['text'], inserted + 'ORIGINAL_BUFFER_MARKER\n')
        self.assertEqual((self.projects[0] / 'note.txt').read_text(), 'ORIGINAL_BUFFER_MARKER\n')
        observed = next(row for row in claude.data('list_buffers', workspace=claude_workspaces['one'])['buffers']
                        if row['name'].endswith('note.txt'))
        self.assertTrue(observed['dirty'])
        self.assertEqual(claude.data('read_buffer', workspace=claude_workspaces['one'], buffer=observed['buffer'],
            expected_revision=observed['revision'], **{'from': 0, 'to': observed['chars']})['text'], read['text'])

        # Revoke only Codex in workspace one through genuine native overlay input.
        standalone.command('context-access codex')
        standalone.wait_output('Agent context access')
        os.write(standalone.fd, b'x')
        time.sleep(.2)
        denied = codex.tool('read_buffer', workspace=codex_workspaces['one'], buffer=original['buffer'],
                           expected_revision=edited['revision'], **{'from': 0, 'to': 1})
        self.assertTrue(denied['isError'])
        self.assertEqual(claude.data('read_buffer', workspace=claude_workspaces['one'], buffer=observed['buffer'],
            expected_revision=observed['revision'], **{'from': 0, 'to': observed['chars']})['text'], read['text'])
        final = codex.data('list_workspaces')['workspaces']
        self.assertFalse(next(row for row in final if Path(row['root']).name == 'one')['readable'])
        self.assertTrue(next(row for row in final if Path(row['root']).name == 'two')['readable'])

    def test_concurrent_appends_from_two_agents_land_whole_without_a_revision(self):
        editor = self.editor(0)
        codex, claude = self.client('codex'), self.client('claude')
        targets = []
        for client in (codex, claude):
            workspace = self.workspaces(client, [self.projects[0]])['one']
            buffers = client.data('list_buffers', workspace=workspace)['buffers']
            targets.append((client, workspace, next(row for row in buffers if row['name'].endswith('note.txt'))))
        errors, results = [], [[], []]

        def write(index):
            client, workspace, row = targets[index]
            try:
                for turn in range(10):
                    results[index].append(client.data('append_buffer', workspace=workspace, buffer=row['buffer'],
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
        observed = next(item for item in client.data('list_buffers', workspace=workspace)['buffers']
                        if item['name'].endswith('note.txt'))
        text = client.data('read_buffer', workspace=workspace, buffer=row['buffer'],
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
            codex.data('append_buffer', workspace=targets[0][1], buffer=targets[0][2]['buffer'],
                       text='never', expected_tail='ORIGINAL_BUFFER_MARKER\n')
        self.assertNotIn('never', client.data('read_buffer', workspace=workspace, buffer=row['buffer'],
            expected_revision=observed['revision'], **{'from': 0, 'to': observed['chars']})['text'])
