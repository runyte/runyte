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
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from test_bridge import MCPClient, PACKAGE, REPO
from native_pty import spawn as spawn_pty
from workspace_readiness import wait_for_workspaces
from runyte_context.client import FRAME_BYTES, decode, encode
from runyte_context.server import PROTOCOL

sys.path.insert(0, str(REPO / 'benchmarks'))
import ptybench

BINARY = os.environ.get('RUNYTE_CONTEXT_TEST_BINARY')
SCOPES = ['terminal_read', 'editor_context_read', 'buffer_edit', 'terminal_propose']
CONTROL = re.compile(rb'\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|P[^\x1b]*\x1b\\)')


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


class NativeEditor:
    def __init__(self, binary, project, config, env, persistent=False):
        self.binary, self.project, self.config, self.env = binary, project, config, env
        self.persistent = persistent
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.output = bytearray()
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
        tail = b''
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
                    combined = tail + data
                    for match in CONTROL.finditer(combined):
                        if match.end() > len(tail):
                            reply = ptybench.terminal_replies(match.group())
                            if reply:
                                os.write(self.fd, reply)
                    tail = combined[-256:]
        except Exception as error:
            self.errors.append(type(error).__name__)

    def wait_output(self, marker, seconds=15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            with self.lock:
                if marker.encode() in CONTROL.sub(b'', bytes(self.output)):
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
        return AssertionError(
            f'Native editor did not display {marker!r}; '
            f'exit={self.process.poll()}, output_bytes={size}, tail={output!r}'
        )

    def command(self, command):
        if self.terminal_input:
            os.write(self.fd, b'\x1c')
            time.sleep(.1)
            self.terminal_input = False
        os.write(self.fd, b'\x1b')
        time.sleep(.1)  # Crossterm must classify Escape separately from Alt-:.
        os.write(self.fd, b':' + command.encode())
        time.sleep(.1)
        os.write(self.fd, b'\r')
        time.sleep(.15)

    def terminal(self, name, marker):
        self.terminal_number += 1
        stop = self.project / ('terminal-stop-' + str(self.terminal_number))
        self.stop_files.append(stop)
        # A data file ends the checked system shell. Nothing executable is written.
        script = 'printf "%s\\n" "$1"; while [ ! -f "$2" ]; do /bin/sleep 0.05; done'
        arguments = ['/bin/sh', '-c', script, 'context-fixture', marker, str(stop)]
        self.command('terminal ' + ' '.join(shlex.quote(value) for value in arguments))
        self.terminal_input = True
        self.wait_output(marker)
        self.command('terminal-rename ' + name)

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


@unittest.skipUnless(BINARY, 'set RUNYTE_CONTEXT_TEST_BINARY to run real editor/bridge integration')
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
