# SPDX-License-Identifier: MPL-2.0
"""Real Runyte + two built-in stdio MCP clients; no accounts or network.

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

from mcp_client import RealMCPClient

REPO = Path(__file__).resolve().parents[2]
from workspace_readiness import wait_for_workspaces

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

    def wait_terminal_exit(self):
        # Quit (even qa!) refuses live terminals. Wait for the host to reap
        # every child, including hidden ones. Rendered exit messages can be
        # hidden by retained command feedback, so inspect semantic liveness.
        # Claude is the fixture identity whose grant is never revoked.
        deadline = time.monotonic() + 15
        client = RealMCPClient(self.binary, self.project.parent, self.env, 'claude')
        try:
            inventory = wait_for_workspaces(
                lambda seconds: client.data('list_workspaces', response_seconds=min(10, seconds)),
                [self.project], seconds=max(0, deadline - time.monotonic()))
            workspace = next(row['workspace'] for row in inventory['workspaces']
                             if Path(row['root']) == self.project)
            while time.monotonic() < deadline:
                terminals = client.data('find_resources', workspace=workspace, kind='terminal',
                                        response_seconds=deadline - time.monotonic())['results']
                if all(not row['live'] for row in terminals):
                    return
                time.sleep(.02)
            raise AssertionError('Fixture terminal exit exceeded deadline')
        finally:
            client.close()

    def close(self):
        failure = None
        try:
            for stop in self.stop_files:
                stop.touch()
            if self.stop_files and not self.persistent and not self.reaped:
                self.wait_terminal_exit()
                self.terminal_input = False
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


class EditorFixture(unittest.TestCase):
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
